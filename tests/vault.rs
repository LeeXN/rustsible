use anyhow::Result;
use std::{
    fs,
    io::Write,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
    process::{Command, Output, Stdio},
};
use tempfile::TempDir;
mod support;
fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rustsible"))
}
fn success(output: Output) -> Output {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn encrypt(path: &Path, password: &Path) {
    success(
        command()
            .args(["vault", "encrypt"])
            .arg(path)
            .arg("--vault-password-file")
            .arg(password)
            .output()
            .unwrap(),
    );
}
#[test]
fn vault_cli_roundtrip_rekey_wrong_password_stdout_and_metadata_are_atomic() -> Result<()> {
    let root = TempDir::new()?;
    let password = root.path().join("password");
    let next = root.path().join("next");
    let wrong = root.path().join("wrong");
    fs::write(&password, "fixture-password\n")?;
    fs::write(&next, "second-password\n")?;
    fs::write(&wrong, "wrong\n")?;
    let path = root.path().join("data");
    let plaintext = b"private-data-canary\0\xff";
    fs::write(&path, plaintext)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o750))?;
    let original = fs::metadata(&path)?;
    success(
        command()
            .args(["vault", "encrypt"])
            .arg(&path)
            .arg("--vault-id")
            .arg(format!("prod@{}", password.display()))
            .output()?,
    );
    let ciphertext = fs::read(&path)?;
    assert!(ciphertext.starts_with(b"$ANSIBLE_VAULT;1.2;AES256;prod\n"));
    assert_ne!(fs::metadata(&path)?.ino(), original.ino());
    assert_eq!(fs::metadata(&path)?.mode() & 0o777, 0o750);
    let encrypted_inode = fs::metadata(&path)?.ino();
    let failed = command()
        .args(["vault", "decrypt"])
        .arg(&path)
        .arg("--vault-password-file")
        .arg(&wrong)
        .output()?;
    assert!(!failed.status.success());
    assert!(!String::from_utf8_lossy(&failed.stderr).contains("private-data-canary"));
    assert_eq!(fs::read(&path)?, ciphertext);
    assert_eq!(fs::metadata(&path)?.ino(), encrypted_inode);
    success(
        command()
            .args(["vault", "rekey"])
            .arg(&path)
            .arg("--vault-password-file")
            .arg(&password)
            .arg("--new-vault-password-file")
            .arg(&next)
            .output()?,
    );
    assert!(fs::read(&path)?.starts_with(b"$ANSIBLE_VAULT;1.2;AES256;prod\n"));
    let viewed = success(
        command()
            .args(["vault", "view"])
            .arg(&path)
            .arg("--vault-password-file")
            .arg(&next)
            .output()?,
    );
    assert_eq!(viewed.stdout, plaintext);
    success(
        command()
            .args(["vault", "decrypt"])
            .arg(&path)
            .arg("--vault-password-file")
            .arg(&next)
            .output()?,
    );
    assert_eq!(fs::read(&path)?, plaintext);
    let metadata = fs::metadata(&path)?;
    assert_eq!(
        (metadata.uid(), metadata.gid(), metadata.mode() & 0o777),
        (original.uid(), original.gid(), 0o750)
    );
    let mut child = command()
        .args(["vault", "encrypt", "-"])
        .arg("--vault-password-file")
        .arg(&password)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child.stdin.take().unwrap().write_all(plaintext)?;
    let encrypted = success(child.wait_with_output()?);
    assert!(encrypted.stdout.starts_with(b"$ANSIBLE_VAULT;1.1;AES256\n"));
    let mut child = command()
        .args(["vault", "decrypt", "-"])
        .arg("--vault-password-file")
        .arg(&password)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child.stdin.take().unwrap().write_all(&encrypted.stdout)?;
    assert_eq!(success(child.wait_with_output()?).stdout, plaintext);
    assert!(!fs::read_dir(root.path())?.any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".rustsible-vault-")));
    Ok(())
}
#[test]
fn vault_playbook_roles_sources_check_diff_and_literal_values_use_memory_only() -> Result<()> {
    let temp = TempDir::new()?;
    let root = temp.path();
    let password = root.join("password");
    fs::write(&password, "fixture-password\n")?;
    let role = root.join("roles/demo");
    for directory in ["tasks", "defaults", "files", "templates"] {
        fs::create_dir_all(role.join(directory))?;
    }
    fs::write(role.join("defaults/main.yml"), "from_role: role-canary\n")?;
    encrypt(&role.join("defaults/main.yml"), &password);
    fs::write(
        role.join("files/message.txt"),
        "literal {{ must_not_execute }}\nsecret-canary\n",
    )?;
    encrypt(&role.join("files/message.txt"), &password);
    fs::write(
        role.join("templates/config.j2"),
        "{{ from_role }}|{{ plain_secret }}|{{ tagged }}",
    )?;
    encrypt(&role.join("templates/config.j2"), &password);
    fs::write(role.join("tasks/main.yml"),"- copy: {src: message.txt, dest: '{{ out }}/copied'}\n  no_log: true\n- template: {src: config.j2, dest: '{{ out }}/rendered'}\n  no_log: true\n")?;
    encrypt(&role.join("tasks/main.yml"), &password);
    fs::write(root.join("vars.yml"), "plain_secret: deploy-canary\n")?;
    encrypt(&root.join("vars.yml"), &password);
    let tagged = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault/reference-1.2.vault"),
    )?;
    fs::create_dir(root.join("group_vars"))?;
    fs::write(
        root.join("group_vars/all.yml"),
        format!(
            "tagged: !vault |\n{}",
            tagged
                .lines()
                .map(|line| format!("  {line}\n"))
                .collect::<String>()
        ),
    )?;
    fs::write(
        root.join("inventory"),
        "localhost ansible_connection=local\n",
    )?;
    fs::create_dir(root.join("out"))?;
    fs::write(root.join("site.yml"),format!("- hosts: all\n  gather_facts: false\n  vars:\n    out: '{}'\n    must_not_execute: SHOULD-NOT-RENDER\n  vars_files: [vars.yml]\n  roles: [demo]\n  tasks:\n    - assert:\n        that: tagged == 'literal {{{{ must_not_execute }}}}\\nsecret-canary\\n'\n      no_log: true\n",root.join("out").display()))?;
    encrypt(&root.join("site.yml"), &password);
    let run = |check| {
        let mut cmd = command();
        cmd.arg("--worker-dir")
            .arg(support::worker_bundle())
            .arg("playbook")
            .arg(root.join("site.yml"))
            .arg("-i")
            .arg(root.join("inventory"))
            .arg("--vault-id")
            .arg(format!("prod@{}", password.display()))
            .arg("--diff")
            .arg("-vvv")
            .current_dir("/");
        if check {
            cmd.arg("--check");
        }
        cmd.output()
    };
    let prediction = success(run(true)?);
    assert_eq!(fs::read_dir(root.join("out"))?.count(), 0);
    let actual = success(run(false)?);
    assert_eq!(
        fs::read_to_string(root.join("out/copied"))?,
        "literal {{ must_not_execute }}\nsecret-canary\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("out/rendered"))?,
        "role-canary|deploy-canary|literal {{ must_not_execute }}\nsecret-canary\n"
    );
    for output in [prediction, actual] {
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        for secret in [
            "role-canary",
            "deploy-canary",
            "secret-canary",
            "fixture-password",
        ] {
            assert!(!all.contains(secret), "Vault value leaked into output");
        }
    }
    Ok(())
}
#[test]
fn unused_vault_scalars_need_no_password_and_failures_do_not_become_undefined() -> Result<()> {
    let temp = TempDir::new()?;
    let root = temp.path();
    let ciphertext = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault/reference-1.1.vault"),
    )?;
    fs::write(
        root.join("inventory"),
        "localhost ansible_connection=local\n",
    )?;
    let vars = format!(
        "  vars:\n    secret: !vault |\n{}",
        ciphertext
            .lines()
            .map(|line| format!("      {line}\n"))
            .collect::<String>()
    );
    fs::write(
        root.join("unused.yml"),
        format!(
            "- hosts: all\n  gather_facts: false\n{vars}  tasks:\n    - debug: {{msg: public}}\n"
        ),
    )?;
    success(
        command()
            .arg("playbook")
            .arg(root.join("unused.yml"))
            .arg("-i")
            .arg(root.join("inventory"))
            .output()?,
    );
    fs::write(root.join("used.yml"),format!("- hosts: all\n  gather_facts: false\n{vars}  tasks:\n    - assert:\n        that: secret is undefined\n"))?;
    let output = command()
        .arg("playbook")
        .arg(root.join("used.yml"))
        .arg("-i")
        .arg(root.join("inventory"))
        .output()?;
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("secret-canary"));
    let provider = root.join("provider");
    let marker = root.join("must-not-execute");
    fs::write(
        &provider,
        format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
    )?;
    fs::set_permissions(&provider, fs::Permissions::from_mode(0o700))?;
    let output = command()
        .arg("playbook")
        .arg(root.join("unused.yml"))
        .arg("-i")
        .arg(root.join("inventory"))
        .arg("--vault-password-file")
        .arg(&provider)
        .output()?;
    assert!(!output.status.success());
    assert!(!marker.exists());
    Ok(())
}

#[path = "support/terminal.rs"]
mod terminal;
#[test]
fn vault_prompts_ids_and_failed_results_keep_keys_and_plaintext_private() -> Result<()> {
    let temp = TempDir::new()?;
    let root = temp.path();
    let encrypted = root.join("encrypted");
    fs::write(&encrypted, b"prompt-payload")?;
    let mut cmd = command();
    cmd.args(["vault", "encrypt"]).arg(&encrypted);
    let result = terminal::run(&mut cmd, &[b"prompt-master-password"])?;
    assert!(result.status.success(), "{}", result.transcript);
    assert!(result.echo_restored);
    let mut cmd = command();
    cmd.args(["vault", "view"])
        .arg(&encrypted)
        .args(["--vault-id", "prod@prompt"]);
    let result = terminal::run(&mut cmd, &[b"prompt-master-password"])?;
    assert!(result.status.success(), "{}", result.transcript);
    assert!(result.echo_restored);
    assert!(result.transcript.contains("prompt-payload"));
    let password = root.join("password");
    fs::write(&password, "fixture-password\n")?;
    let first = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault/reference-1.1.vault"),
    )?;
    fs::write(
        root.join("inventory"),
        "localhost ansible_connection=local\n",
    )?;
    fs::write(root.join("site.yml"),format!("- hosts: all\n  gather_facts: false\n  vars:\n    secret: !vault |\n{}  tasks:\n    - fail:\n        msg: '{{{{ secret }}}}'\n",first.lines().map(|line|format!("      {line}\n")).collect::<String>()))?;
    let result = command()
        .arg("playbook")
        .arg(root.join("site.yml"))
        .arg("-i")
        .arg(root.join("inventory"))
        .arg("--vault-password-file")
        .arg(&password)
        .arg("-vvv")
        .output()?;
    assert!(!result.status.success());
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!text.contains("secret-canary"));
    assert!(!text.contains("fixture-password"));
    assert!(text.contains("failed=1"));
    Ok(())
}
#[test]
fn vault_async_results_remain_private_when_queried_without_password_sources() -> Result<()> {
    let temp = TempDir::new()?;
    let root = temp.path();
    let password = root.join("password");
    fs::write(&password, "fixture-password\n")?;
    fs::write(
        root.join("inventory"),
        format!(
            "localhost ansible_connection=local ansible_async_dir={}\n",
            root.join("jobs").display()
        ),
    )?;
    fs::write(root.join("vars.yml"), "secret: async-private-canary\n")?;
    encrypt(&root.join("vars.yml"), &password);
    fs::write(
        root.join("site.yml"),
        format!(
            r#"
- hosts: all
  gather_facts: false
  vars_files: [vars.yml]
  tasks:
    - command:
        argv: [/bin/echo, '{{{{ secret }}}}']
      async: 20
      poll: 0
      register: launched
    - copy:
        content: '{{{{ launched.ansible_job_id }}}}'
        dest: '{}'
"#,
            root.join("jid").display()
        ),
    )?;
    let output = success(
        command()
            .arg("--worker-dir")
            .arg(support::worker_bundle())
            .arg("playbook")
            .arg(root.join("site.yml"))
            .arg("-i")
            .arg(root.join("inventory"))
            .arg("--vault-password-file")
            .arg(&password)
            .output()?,
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("async-private-canary"));
    let jid = fs::read_to_string(root.join("jid"))?;
    let start = std::time::Instant::now();
    loop {
        let output = success(
            command()
                .arg("--worker-dir")
                .arg(support::worker_bundle())
                .args(["ad-hoc", "localhost", "-i"])
                .arg(root.join("inventory"))
                .args(["-m", "async_status", "-a"])
                .arg(format!("{{\"jid\":\"{jid}\"}}"))
                .output()?,
        );
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(!text.contains("async-private-canary"));
        let result: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("jobs").join(&jid).join("status.json"))?)?;
        if result["finished"] == true {
            assert!(text.contains("censored"));
            break;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(10));
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    success(
        command()
            .arg("--worker-dir")
            .arg(support::worker_bundle())
            .args(["ad-hoc", "localhost", "-i"])
            .arg(root.join("inventory"))
            .args(["-m", "async_status", "-a"])
            .arg(format!("{{\"jid\":\"{jid}\",\"mode\":\"cleanup\"}}"))
            .output()?,
    );
    assert!(!root.join("jobs").join(&jid).exists());
    Ok(())
}

#[test]
fn multiple_vault_ids_are_hints_and_new_outputs_are_private() -> Result<()> {
    let temp = TempDir::new()?;
    let root = temp.path();
    let dev = root.join("dev");
    let prod = root.join("prod");
    let wrong = root.join("wrong");
    fs::write(&dev, "dev-password\n")?;
    fs::write(&prod, "prod-password\n")?;
    fs::write(&wrong, "wrong-password\n")?;
    let input = root.join("plain");
    let output = root.join("encrypted");
    fs::write(&input, "multi-id-payload")?;
    success(
        command()
            .arg("--vault-id")
            .arg(format!("dev@{}", dev.display()))
            .args(["vault", "encrypt"])
            .arg(&input)
            .arg("--vault-id")
            .arg(format!("prod@{}", prod.display()))
            .args(["--encrypt-vault-id", "prod", "--output"])
            .arg(&output)
            .output()?,
    );
    assert_eq!(fs::read_to_string(&input)?, "multi-id-payload");
    assert_eq!(fs::metadata(&output)?.mode() & 0o777, 0o600);
    assert!(fs::read(&output)?.starts_with(b"$ANSIBLE_VAULT;1.2;AES256;prod\n"));
    let result = success(
        command()
            .args(["vault", "view"])
            .arg(&output)
            .arg("--vault-id")
            .arg(format!("prod@{}", wrong.display()))
            .arg("--vault-id")
            .arg(format!("dev@{}", prod.display()))
            .output()?,
    );
    assert_eq!(result.stdout, b"multi-id-payload");
    let before_root = root.join("before-root");
    success(
        command()
            .arg("--vault-id")
            .arg(format!("dev@{}", dev.display()))
            .args(["vault", "encrypt"])
            .arg(&input)
            .arg("--vault-id")
            .arg(format!("prod@{}", prod.display()))
            .args(["--encrypt-vault-id", "dev", "--output"])
            .arg(&before_root)
            .output()?,
    );
    assert!(fs::read(&before_root)?.starts_with(b"$ANSIBLE_VAULT;1.2;AES256;dev\n"));
    let inode = fs::metadata(&output)?.ino();
    std::os::unix::fs::symlink(&output, root.join("link"))?;
    let failed = command()
        .args(["vault", "decrypt"])
        .arg(root.join("link"))
        .arg("--vault-password-file")
        .arg(&prod)
        .output()?;
    assert!(!failed.status.success());
    assert_eq!(fs::metadata(&output)?.ino(), inode);
    let failed = command()
        .args(["vault", "encrypt"])
        .arg(&input)
        .arg("--vault-password-file")
        .arg(&dev)
        .arg("--output")
        .arg(root.join("."))
        .output()?;
    assert!(!failed.status.success());
    Ok(())
}
