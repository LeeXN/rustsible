use anyhow::Result;
use std::{fs, process::Command};
use tempfile::TempDir;
mod support;
fn run(root: &std::path::Path, options: &[&str]) -> Result<std::process::Output> {
    Ok(Command::new(env!("CARGO_BIN_EXE_rustsible"))
        .arg("--worker-dir")
        .arg(support::worker_bundle())
        .arg("playbook")
        .arg(root.join("site.yml"))
        .arg("-i")
        .arg(root.join("inventory"))
        .args(options)
        .output()?)
}
#[test]
fn tags_select_static_dynamic_blocks_templated_values_and_notified_handlers() -> Result<()> {
    for (only, skip, expected) in [
        (
            "alpha",
            "",
            vec![
                "static",
                "child",
                "applied",
                "block",
                "templated",
                "handled",
            ],
        ),
        (
            "all",
            "alpha,unavailable",
            vec!["untagged", "untagged-child"],
        ),
        ("never", "", vec!["never"]),
        ("untagged", "always", vec!["untagged"]),
    ] {
        let temporary = TempDir::new()?;
        let root = temporary.path();
        let output = root.join("out");
        fs::create_dir(&output)?;
        fs::write(
            root.join("inventory"),
            "localhost ansible_connection=local\n",
        )?;
        fs::write(
            root.join("static.yml"),
            "- copy: {content: selected, dest: '{{ out }}/static'}\n  notify: finish\n",
        )?;
        fs::write(root.join("children.yml"), "- copy: {content: selected, dest: '{{ out }}/child'}\n  tags: alpha\n- copy: {content: selected, dest: '{{ out }}/untagged-child'}\n")?;
        fs::write(
            root.join("apply.yml"),
            "- copy: {content: selected, dest: '{{ out }}/applied'}\n",
        )?;
        fs::write(
            root.join("site.yml"),
            format!(
                r#"
- hosts: all
  gather_facts: false
  vars:
    out: '{}'
    chosen: [alpha]
  tasks:
    - import_tasks: static.yml
      tags: alpha
    - include_tasks: children.yml
      tags: always
    - include_tasks:
        file: apply.yml
        apply:
          tags: alpha
      tags: always
    - block:
        - copy: {{content: selected, dest: '{{{{ out }}}}/block'}}
      tags: alpha
    - copy: {{content: selected, dest: '{{{{ out }}}}/templated'}}
      tags: '{{{{ chosen }}}}'
    - copy: {{content: selected, dest: '{{{{ out }}}}/untagged'}}
    - copy: {{content: selected, dest: '{{{{ out }}}}/never'}}
      tags: never
    - include_tasks: must-not-exist.yml
      tags: unavailable
  handlers:
    - name: finish
      copy: {{content: selected, dest: '{{{{ out }}}}/handled'}}
      tags: never
"#,
                output.display()
            ),
        )?;
        let actual = run(root, &["--tags", only, "--skip-tags", skip])?;
        assert!(
            actual.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&actual.stdout),
            String::from_utf8_lossy(&actual.stderr)
        );
        let mut actual = fs::read_dir(&output)?
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        actual.sort();
        let mut expected = expected;
        expected.sort();
        assert_eq!(actual, expected, "{only}/{skip}");
    }
    Ok(())
}
#[test]
fn inspection_is_json_and_never_requires_workers_or_connects_to_hosts() -> Result<()> {
    let temporary = TempDir::new()?;
    let root = temporary.path();
    fs::write(
        root.join("inventory"),
        "one ansible_host=must-not-resolve.invalid\ntwo ansible_host=must-not-resolve.invalid\n",
    )?;
    fs::write(root.join("site.yml"), "- hosts: all\n  tasks:\n    - debug: {msg: secret-argument-canary}\n      name: selected\n      tags: alpha\n    - include_tasks: missing.yml\n      tags: beta\n")?;
    let run = |flags: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_rustsible"))
            .arg("--worker-dir")
            .arg(root.join("missing-workers"))
            .arg("playbook")
            .arg(root.join("site.yml"))
            .arg("-i")
            .arg(root.join("inventory"))
            .args(flags)
            .output()
    };
    for flags in [
        vec!["--syntax-check"],
        vec!["--list-hosts", "--limit", "one"],
        vec!["--list-tasks", "--tags", "alpha"],
    ] {
        let actual = run(&flags)?;
        assert!(
            actual.status.success(),
            "{}",
            String::from_utf8_lossy(&actual.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&actual.stdout)?;
        assert!(!String::from_utf8_lossy(&actual.stdout).contains("secret-argument-canary"));
        if flags[0] == "--list-hosts" {
            assert_eq!(json[0]["hosts"], serde_json::json!(["one"]));
        }
        if flags[0] == "--list-tasks" {
            assert_eq!(json[0]["tasks"].as_array().unwrap().len(), 1);
            assert_eq!(json[0]["tasks"][0]["name"], "selected");
        }
    }
    fs::write(
        root.join("site.yml"),
        "- hosts: all\n  tasks:\n    - copy: {content: x, dest: /unreachable, unknown: true}\n",
    )?;
    let invalid = run(&["--syntax-check"])?;
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("Unsupported copy parameter"));
    Ok(())
}

#[path = "support/terminal.rs"]
mod terminal;
#[test]
fn password_prompts_hide_input_restore_echo_and_do_not_expose_variables() -> Result<()> {
    use std::os::unix::process::CommandExt;
    let temporary = TempDir::new()?;
    let root = temporary.path();
    fs::write(
        root.join("inventory"),
        "localhost ansible_connection=local\n",
    )?;
    fs::write(root.join("site.yml"),"- hosts: all\n  gather_facts: false\n  tasks:\n    - assert:\n        that:\n          - ansible_password is undefined\n          - ansible_become_password is undefined\n")?;
    let command = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rustsible"));
        command
            .arg("playbook")
            .arg(root.join("site.yml"))
            .arg("-i")
            .arg(root.join("inventory"));
        command
    };
    let mut actual = command();
    actual.args(["-k", "-K"]);
    let output = terminal::run(
        &mut actual,
        &[b"ssh-password-canary", b"sudo-password-canary"],
    )?;
    assert!(output.status.success(), "{}", output.transcript);
    assert!(output.echo_restored);
    let mut cancelled = command();
    cancelled.arg("-k");
    let output = terminal::run(&mut cancelled, &[b"\x03"])?;
    assert!(!output.status.success());
    assert!(output.echo_restored);
    assert!(!output.transcript.contains("TASK ["));
    let mut noninteractive = command();
    noninteractive.arg("-k");
    unsafe {
        noninteractive.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let output = noninteractive.output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("controlling terminal"));
    let output = command().args(["--syntax-check", "-k", "-K"]).output()?;
    assert!(output.status.success());
    Ok(())
}

#[test]
fn diff_is_bounded_and_no_log_censors_display_while_register_retains_data() -> Result<()> {
    let temporary = TempDir::new()?;
    let root = temporary.path();
    fs::write(
        root.join("inventory"),
        "localhost ansible_connection=local\n",
    )?;
    fs::write(root.join("visible"), "old visible")?;
    fs::write(
        root.join("site.yml"),
        format!(
            r#"
- hosts: all
  gather_facts: false
  tasks:
    - copy:
        content: new visible
        dest: '{}'
    - copy:
        content: secret-diff-canary
        dest: '{}'
      no_log: true
      register: hidden
    - assert:
        that: hidden.diff.after.content == 'secret-diff-canary'
      no_log: true
"#,
            root.join("visible").display(),
            root.join("hidden").display()
        ),
    )?;
    for options in [vec!["--check", "--diff"], vec!["--diff"]] {
        let actual = run(root, &options)?;
        let text = String::from_utf8_lossy(&actual.stdout);
        assert!(
            actual.status.success(),
            "{}\n{}",
            text,
            String::from_utf8_lossy(&actual.stderr)
        );
        assert!(text.contains("old visible") && text.contains("new visible"));
        assert!(!text.contains("secret-diff-canary"));
        if options.contains(&"--check") {
            assert_eq!(fs::read_to_string(root.join("visible"))?, "old visible");
            assert!(!root.join("hidden").exists());
        }
    }
    assert_eq!(
        fs::read_to_string(root.join("hidden"))?,
        "secret-diff-canary"
    );
    Ok(())
}

#[test]
fn filtered_rescue_does_not_clear_failure_or_allow_later_modifications() -> Result<()> {
    let temporary = TempDir::new()?;
    let root = temporary.path();
    fs::write(
        root.join("inventory"),
        "localhost ansible_connection=local\n",
    )?;
    fs::write(
        root.join("site.yml"),
        format!(
            r#"
- hosts: all
  gather_facts: false
  tasks:
    - block:
        - fail: {{msg: expected}}
          tags: alpha
      rescue:
        - copy: {{content: rescued, dest: '{}'}}
      always:
        - copy: {{content: always, dest: '{}'}}
          tags: alpha
    - copy: {{content: after, dest: '{}'}}
      tags: alpha
"#,
            root.join("rescued").display(),
            root.join("always").display(),
            root.join("after").display()
        ),
    )?;
    let result = run(root, &["--tags", "alpha"])?;
    assert!(!result.status.success());
    assert!(!root.join("rescued").exists());
    assert!(!root.join("after").exists());
    assert_eq!(fs::read_to_string(root.join("always"))?, "always");
    assert!(String::from_utf8_lossy(&result.stdout).contains("rescued=0"));
    Ok(())
}

#[test]
fn tag_selection_can_skip_implicit_facts_without_manufacturing_fact_values() -> Result<()> {
    for (play_tags, skip) in [("", "always"), ("  tags: beta\n", "")] {
        let temporary = TempDir::new()?;
        let root = temporary.path();
        fs::write(
            root.join("inventory"),
            "target ansible_host=must-not-resolve.invalid ansible_ssh_timeout=1\n",
        )?;
        fs::write(root.join("site.yml"),format!("- hosts: all\n{play_tags}  tasks:\n    - assert:\n        that: ansible_facts | length == 0\n      tags: alpha\n"))?;
        let result = run(root, &["--tags", "alpha", "--skip-tags", skip])?;
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!String::from_utf8_lossy(&result.stdout).contains("Gathering Facts"));
    }
    Ok(())
}
