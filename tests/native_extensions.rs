use anyhow::{Context, Result};
use rustsible_protocol::{
    extension::{Call, Capabilities, Definition, Kind, Parameter, ParameterType as T, Reply},
    ModuleResult,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};
fn fixture() -> Result<PathBuf> {
    let executable = std::env::current_exe()?;
    Ok(executable
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("rustsible-extension-fixture"))
}
fn definition(name: &str, params: &[(&str, T, bool)]) -> Definition {
    Definition {
        name: name.into(),
        kind: Kind::Module,
        capabilities: Capabilities {
            check: true,
            diff: name.ends_with("write"),
            asynchronous: !name.ends_with("crash"),
            mutates: true,
        },
        parameters: params
            .iter()
            .map(|(name, kind, required)| {
                (
                    (*name).into(),
                    Parameter {
                        kind: *kind,
                        required: *required,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>(),
    }
}
fn invocation(
    name: &str,
    args: serde_json::Value,
    check: bool,
    timeout_ms: u64,
) -> Result<rustsible_native::rpc::Invocation> {
    let definition = definition(
        name,
        if name.ends_with("write") {
            &[("path", T::String, true), ("content", T::String, true)]
        } else if name.ends_with("timeout") {
            &[("path", T::String, true)]
        } else {
            &[]
        },
    );
    let program = fixture()?;
    let sha256 = rustsible_protocol::sha256(
        &mut fs::File::open(&program).context("Build the workspace before extension tests")?,
    )?;
    let call = Call::new(
        &definition,
        serde_yaml::to_value(args)?,
        check,
        false,
        timeout_ms,
    );
    Ok(rustsible_native::rpc::Invocation {
        program,
        sha256,
        definition,
        call,
    })
}
fn invoke(request: &rustsible_native::rpc::Invocation, env: Option<&str>) -> Result<ModuleResult> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rustsible"));
    command
        .arg("--native-extension-runner")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(name) = env {
        command.env(name, "1");
    }
    let mut child = command.spawn()?;
    let mut input = child.stdin.take().unwrap();
    rustsible_protocol::write_frame(&mut input, request, rustsible_protocol::MAX_REQUEST_BYTES)?;
    let response: Reply = rustsible_protocol::read_frame(
        child.stdout.as_mut().unwrap(),
        rustsible_protocol::MAX_RESPONSE_BYTES,
    )?;
    drop(input);
    let output = child.wait_with_output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    response.validate()
}
#[test]
fn native_handshake_schema_check_and_parameter_delivery_are_ordered() -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let path = temporary.path().join("data");
    let request = invocation(
        "fixture.native.write",
        serde_json::json!({"path":path,"content":"native"}),
        true,
        5000,
    )?;
    let result = invoke(&request, None)?;
    assert!(!result.failed && result.changed, "{}", result.msg);
    assert!(!path.exists());
    let mut request = request;
    request.call.check = false;
    let result = invoke(&request, None)?;
    assert!(!result.failed && result.changed, "{}", result.msg);
    assert_eq!(fs::read(&path)?, b"native");
    assert!(!invoke(&request, None)?.changed);
    fs::remove_file(&path)?;
    let result = invoke(&request, Some("RUSTSIBLE_FIXTURE_BAD_HELLO"))?;
    assert!(result.failed && !result.uncertain, "{}", result.msg);
    assert!(!path.exists());
    let result = invoke(&request, Some("RUSTSIBLE_FIXTURE_RAW_STDOUT"))?;
    assert!(result.failed);
    assert!(!path.exists());
    Ok(())
}
#[test]
fn native_extension_crashes_and_timeout_reap_detached_descendants() -> Result<()> {
    let result = invoke(
        &invocation("fixture.native.crash", serde_json::json!({}), false, 5000)?,
        None,
    )?;
    assert!(result.failed && result.uncertain, "{}", result.msg);
    let temporary = tempfile::tempdir()?;
    let path = temporary.path().join("pid");
    let result = invoke(
        &invocation(
            "fixture.native.timeout",
            serde_json::json!({"path":path}),
            false,
            // Cold native ARM runners also hash and start the controller and
            // peer under concurrent test load. Leave startup headroom while
            // staying below the fixture's 30-second business operation.
            15000,
        )?,
        None,
    )?;
    assert!(result.failed && result.uncertain, "{}", result.msg);
    let pid = fs::read_to_string(&path)
        .context("Detached fixture child did not start before the extension deadline")?;
    assert!(!Path::new(&format!("/proc/{}", pid.trim())).exists());
    std::thread::sleep(Duration::from_millis(5));
    Ok(())
}

fn collection(root: &Path) -> Result<PathBuf> {
    let directory = root.join("collections/ansible_collections/fixture/native");
    fs::create_dir_all(directory.join("bin"))?;
    let program = fixture()?;
    fs::copy(&program, directory.join("bin/fixture"))?;
    let sha256 = rustsible_protocol::sha256(&mut fs::File::open(&program)?)?;
    let metadata = rustsible_protocol::extension::Collection {
        format: 1,
        namespace: "fixture".into(),
        name: "native".into(),
        version: "0.1.0".into(),
        extensions: vec![rustsible_protocol::extension::Entry {
            definition: definition(
                "fixture.native.write",
                &[("path", T::String, true), ("content", T::String, true)],
            ),
            artifacts: BTreeMap::from([(
                std::env::consts::ARCH.into(),
                rustsible_protocol::extension::Artifact {
                    path: "bin/fixture".into(),
                    sha256,
                },
            )]),
        }],
    };
    fs::write(
        directory.join("rustsible-collection.json"),
        serde_json::to_vec(&metadata)?,
    )?;
    Ok(root.join("collections"))
}
mod support;
#[test]
fn configured_native_modules_run_through_ad_hoc_playbook_and_async() -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let root = temporary.path();
    let collections = collection(root)?;
    let path = root.join("native");
    let inventory = root.join("inventory");
    fs::write(
        &inventory,
        format!(
            "localhost ansible_connection=local ansible_async_dir={}\n",
            root.join("jobs").display()
        ),
    )?;
    let args = serde_json::json!({"path":path,"content":"configured-native"}).to_string();
    let run = |flags: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_rustsible"))
            .arg("--worker-dir")
            .arg(support::worker_bundle())
            .args(["ad-hoc", "localhost", "-i"])
            .arg(&inventory)
            .arg("--collections-path")
            .arg(&collections)
            .args(["-m", "fixture.native.write", "-a"])
            .arg(&args)
            .args(flags)
            .output()
    };
    let predicted = run(&["--check"])?;
    assert!(
        predicted.status.success(),
        "{}{}",
        String::from_utf8_lossy(&predicted.stdout),
        String::from_utf8_lossy(&predicted.stderr)
    );
    assert!(!path.exists());
    let actual = run(&[])?;
    assert!(
        actual.status.success(),
        "{}{}",
        String::from_utf8_lossy(&actual.stdout),
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(fs::read_to_string(&path)?, "configured-native");
    fs::remove_file(&path)?;
    let predicted = run(&["--check", "--diff"])?;
    assert!(predicted.status.success());
    assert!(
        String::from_utf8_lossy(&predicted.stdout).contains("native_fixture"),
        "{}",
        String::from_utf8_lossy(&predicted.stdout)
    );
    assert!(!path.exists());
    let actual = run(&["-B", "20", "-P", "1"])?;
    assert!(
        actual.status.success(),
        "{}{}",
        String::from_utf8_lossy(&actual.stdout),
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(fs::read_to_string(&path)?, "configured-native");
    assert_eq!(fs::read_dir(root.join("jobs"))?.count(), 0);
    fs::write(root.join("site.yml"),format!("- hosts: all\n  gather_facts: false\n  tasks:\n    - fixture.native.write:\n        path: '{}'\n        content: playbook-native\n      register: written\n    - assert:\n        that: written.changed\n",path.display()))?;
    let actual = Command::new(env!("CARGO_BIN_EXE_rustsible"))
        .arg("--worker-dir")
        .arg(support::worker_bundle())
        .arg("playbook")
        .arg(root.join("site.yml"))
        .arg("-i")
        .arg(&inventory)
        .arg("--collections-path")
        .arg(&collections)
        .output()?;
    assert!(
        actual.status.success(),
        "{}{}",
        String::from_utf8_lossy(&actual.stdout),
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(fs::read_to_string(&path)?, "playbook-native");
    let large = serde_json::json!({"path":path,"content":"é".repeat(40000)}).to_string();
    let output = Command::new(env!("CARGO_BIN_EXE_rustsible"))
        .arg("--worker-dir")
        .arg(support::worker_bundle())
        .args(["ad-hoc", "localhost", "-i"])
        .arg(&inventory)
        .arg("--collections-path")
        .arg(&collections)
        .args(["-m", "fixture.native.write", "-a"])
        .arg(large)
        .arg("--diff")
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::metadata(&path)?.len(), 80000);
    assert!(output.stdout.len() < 4096);
    assert!(String::from_utf8_lossy(&output.stdout).contains("content_omitted"));
    Ok(())
}

#[test]
fn native_inventory_lookup_filter_and_collection_roles_keep_external_text_literal() -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let root = temporary.path();
    let directory = root.join("collections/ansible_collections/example/native");
    fs::create_dir_all(directory.join("bin"))?;
    fs::create_dir_all(directory.join("roles/demo/tasks"))?;
    let program = fixture()?;
    fs::copy(&program, directory.join("bin/native"))?;
    let digest = rustsible_protocol::sha256(&mut fs::File::open(&program)?)?;
    let metadata = rustsible_protocol::extension::Collection {
        format: 1,
        namespace: "example".into(),
        name: "native".into(),
        version: "0.1.0".into(),
        extensions: rustsible_native_example::catalog()
            .into_iter()
            .map(|definition| rustsible_protocol::extension::Entry {
                definition,
                artifacts: BTreeMap::from([(
                    std::env::consts::ARCH.into(),
                    rustsible_protocol::extension::Artifact {
                        path: "bin/native".into(),
                        sha256: digest.clone(),
                    },
                )]),
            })
            .collect(),
    };
    fs::write(
        directory.join("rustsible-collection.json"),
        serde_json::to_vec(&metadata)?,
    )?;
    fs::write(directory.join("roles/demo/tasks/main.yml"),"- probe: {path: /etc/passwd}\n  register: role_probe\n- ansible.builtin.assert:\n    that:\n      - role_probe.probe.exists\n      - ansible_role_name == 'example.native.demo'\n      - role_name == 'demo'\n      - ansible_collection_name == 'example.native'\n")?;
    let literal = "literal {{ MUST_NOT_RENDER }}";
    fs::write(
        root.join("hosts.json"),
        serde_json::to_vec(
            &serde_json::json!({"all":{"hosts":{"native":{"ansible_connection":"local","external_text":literal}}}}),
        )?,
    )?;
    fs::write(
        root.join("inventory.yml"),
        format!(
            "plugin: example.native.inventory\npath: '{}'\n",
            root.join("hosts.json").display()
        ),
    )?;
    fs::write(
        root.join("record.json"),
        serde_json::to_vec(&serde_json::json!({"number":42,"literal":literal}))?,
    )?;
    fs::write(
        root.join("site.yml"),
        format!(
            r#"
- hosts: all
  gather_facts: false
  collections: [example.native]
  vars:
    record: "{{{{ lookup('example.native.record', '{}') }}}}"
  roles: [example.native.demo]
  tasks:
    - copy: {{content: '{{{{ external_text }}}}', dest: '{}'}}
    - copy: {{content: '{{{{ record.literal }}}}', dest: '{}'}}
    - copy: {{content: "{{{{ 'Hello World' | example.native.slug }}}}", dest: '{}'}}
    - probe: {{path: /etc/passwd}}
      register: observed
    - assert:
        that:
          - observed.probe.exists
          - record.number == 42
"#,
            root.join("record.json").display(),
            root.join("inventory-result").display(),
            root.join("lookup-result").display(),
            root.join("filter-result").display()
        ),
    )?;
    let output = Command::new(env!("CARGO_BIN_EXE_rustsible"))
        .arg("--worker-dir")
        .arg(support::worker_bundle())
        .arg("playbook")
        .arg(root.join("site.yml"))
        .arg("-i")
        .arg(root.join("inventory.yml"))
        .arg("--collections-path")
        .arg(root.join("collections"))
        .output()?;
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read_to_string(root.join("inventory-result"))?, literal);
    assert_eq!(fs::read_to_string(root.join("lookup-result"))?, literal);
    assert_eq!(
        fs::read_to_string(root.join("filter-result"))?,
        "hello-world"
    );
    Ok(())
}

#[test]
fn native_packages_require_installation_or_configuration_and_detect_tampering() -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let root = temporary.path();
    let collections = collection(root)?;
    let source = collections.join("ansible_collections/fixture/native");
    let archive = root.join("fixture.tar.gz");
    let installed = root.join("installed");
    let path = root.join("value");
    let inventory = root.join("inventory");
    fs::write(&inventory, "localhost ansible_connection=local\n")?;
    let pack = Command::new(env!("CARGO_BIN_EXE_rustsible"))
        .args(["collection", "pack"])
        .arg(&source)
        .arg("--output")
        .arg(&archive)
        .output()?;
    assert!(
        pack.status.success(),
        "{}",
        String::from_utf8_lossy(&pack.stderr)
    );
    let verified = Command::new(env!("CARGO_BIN_EXE_rustsible"))
        .args(["collection", "verify"])
        .arg(&archive)
        .output()?;
    assert!(
        verified.status.success(),
        "{}",
        String::from_utf8_lossy(&verified.stderr)
    );
    let install = |flags: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_rustsible"))
            .args(["collection", "install"])
            .arg(&archive)
            .arg("--path")
            .arg(&installed)
            .args(flags)
            .output()
    };
    let output = install(&[])?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let args = serde_json::json!({"path":path,"content":"package"}).to_string();
    let run = |directory: &Path, project: &Path| {
        Command::new(env!("CARGO_BIN_EXE_rustsible"))
            .arg("--worker-dir")
            .arg(support::worker_bundle())
            .args(["ad-hoc", "localhost", "-i"])
            .arg(&inventory)
            .arg("--collections-path")
            .arg(directory)
            .args(["-m", "fixture.native.write", "-a"])
            .arg(&args)
            .current_dir(project)
            .output()
    };
    let output = run(&installed, root)?;
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read_to_string(&path)?, "package");
    let output = install(&[])?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let installed_collection = installed.join("ansible_collections/fixture/native");
    fs::create_dir(installed_collection.join("unmanaged-empty"))?;
    assert!(!install(&[])?.status.success());
    assert!(installed_collection.join("unmanaged-empty").is_dir());
    fs::remove_dir(installed_collection.join("unmanaged-empty"))?;
    fs::write(installed_collection.join("unmanaged"), "keep unrelated")?;
    assert!(!install(&[])?.status.success());
    assert_eq!(
        fs::read_to_string(installed_collection.join("unmanaged"))?,
        "keep unrelated"
    );
    fs::remove_file(installed_collection.join("unmanaged"))?;
    fs::write(installed_collection.join("bin/fixture"), b"corrupted")?;
    assert!(!run(&installed, root)?.status.success());
    // A project-local manifest/ELF is passive unless the operator explicitly
    // configures its search path; the package cannot authorize itself.
    fs::write(root.join("site.yml"),format!("- hosts: all\n  gather_facts: false\n  tasks:\n    - fixture.native.write: {{path: '{}', content: forbidden}}\n",path.display()))?;
    let output = Command::new(env!("CARGO_BIN_EXE_rustsible"))
        .arg("playbook")
        .arg(root.join("site.yml"))
        .arg("-i")
        .arg(&inventory)
        .current_dir(root)
        .output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("explicit installation"));
    assert_eq!(fs::read_to_string(&path)?, "package");
    Ok(())
}
#[test]
fn default_native_store_authorization_cannot_be_forged_inside_a_repository() -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let root = temporary.path();
    let collections = collection(root)?;
    let source = collections.join("ansible_collections/fixture/native");
    let archive = root.join("fixture.tar.gz");
    let home = root.join("isolated-home");
    fs::create_dir(&home)?;
    let output = Command::new(env!("CARGO_BIN_EXE_rustsible"))
        .args(["collection", "pack"])
        .arg(&source)
        .arg("--output")
        .arg(&archive)
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // HOME belongs only to the child test process and keeps installation trust
    // state out of the operator's real home/configuration.
    let output = Command::new(env!("CARGO_BIN_EXE_rustsible"))
        .env("HOME", &home)
        .args(["collection", "install"])
        .arg(&archive)
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let target = root.join("value");
    let inventory = root.join("inventory");
    fs::write(&inventory, "localhost ansible_connection=local\n")?;
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_rustsible"))
            .env("HOME", &home)
            .arg("--worker-dir")
            .arg(support::worker_bundle())
            .args(["ad-hoc", "localhost", "-i"])
            .arg(&inventory)
            .args(["-m", "fixture.native.write", "-a"])
            .arg(serde_json::json!({"path":target,"content":"authorized"}).to_string())
            .output()
    };
    let output = run()?;
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    fs::remove_file(home.join(".rustsible/native-trust.json"))?;
    let output = run()?;
    assert!(!output.status.success());
    assert_eq!(fs::read_to_string(target)?, "authorized");
    Ok(())
}
