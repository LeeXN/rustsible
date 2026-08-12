use anyhow::Result;
use rustsible::inventory;
use rustsible::playbook;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use tempfile::{NamedTempFile, TempDir};

fn fixture_file(contents: &str) -> NamedTempFile {
    let mut file = NamedTempFile::new().expect("create temporary fixture");
    file.write_all(contents.as_bytes())
        .expect("write temporary fixture");
    file
}

fn local_inventory() -> NamedTempFile {
    fixture_file("[local]\nlocalhost ansible_connection=local\n")
}

#[test]
fn test_integration_simple_playbook_local() -> Result<()> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let inventory_path = manifest_dir.join("tests/fixtures/inventory/simple.ini");
    let playbook_path = manifest_dir.join("tests/fixtures/playbooks/simple.yml");

    anyhow::ensure!(inventory_path.exists(), "inventory fixture is missing");
    anyhow::ensure!(playbook_path.exists(), "playbook fixture is missing");

    println!("Loading inventory from: {:?}", inventory_path);
    let inventory = inventory::parse(inventory_path.to_str().unwrap())?;

    println!("Executing playbook: {:?}", playbook_path);
    playbook::execute(playbook_path.to_str().unwrap(), &inventory)?;

    Ok(())
}

#[test]
fn test_check_mode_never_runs_command() -> Result<()> {
    let inventory_file = local_inventory();
    let output_dir = TempDir::new()?;
    let marker = output_dir.path().join("check-mode-marker");
    let playbook_file = fixture_file(&format!(
        "---\n- name: check mode\n  hosts: localhost\n  tasks:\n    - name: must not run\n      command: touch {}\n",
        marker.display()
    ));
    let inventory = inventory::parse(inventory_file.path().to_str().unwrap())?;
    let options = playbook::ExecutionOptions {
        limit: None,
        check_mode: true,
        ..playbook::ExecutionOptions::default()
    };

    playbook::execute_with_options(playbook_file.path().to_str().unwrap(), &inventory, &options)?;

    assert!(!marker.exists(), "check mode created a real file");
    Ok(())
}

#[test]
fn test_limit_that_matches_nothing_is_an_error() -> Result<()> {
    let inventory_file = local_inventory();
    let playbook_file = fixture_file(
        "---\n- name: limited\n  hosts: all\n  tasks:\n    - name: output\n      debug:\n        msg: hello\n",
    );
    let inventory = inventory::parse(inventory_file.path().to_str().unwrap())?;
    let options = playbook::ExecutionOptions {
        limit: Some("does-not-exist".to_string()),
        check_mode: false,
        ..playbook::ExecutionOptions::default()
    };

    let result = playbook::execute_with_options(
        playbook_file.path().to_str().unwrap(),
        &inventory,
        &options,
    );

    assert!(result.is_err());
    Ok(())
}

#[test]
fn test_failed_playbook_returns_nonzero_exit_status() -> Result<()> {
    let inventory_file = local_inventory();
    let playbook_file = fixture_file(
        "---\n- name: failure\n  hosts: localhost\n  tasks:\n    - name: fail deliberately\n      command: 'false'\n",
    );

    let status = Command::new(env!("CARGO_BIN_EXE_rustsible"))
        .arg("playbook")
        .arg(playbook_file.path())
        .arg("-i")
        .arg(inventory_file.path())
        .status()?;

    assert!(!status.success());
    Ok(())
}

#[test]
fn test_unknown_module_returns_nonzero_exit_status() -> Result<()> {
    let inventory_file = local_inventory();
    let playbook_file = fixture_file(
        "---\n- name: unknown module\n  hosts: localhost\n  tasks:\n    - name: must fail\n      definitely_not_a_module:\n        value: true\n",
    );

    let status = Command::new(env!("CARGO_BIN_EXE_rustsible"))
        .arg("playbook")
        .arg(playbook_file.path())
        .arg("-i")
        .arg(inventory_file.path())
        .status()?;

    assert!(!status.success());
    Ok(())
}

#[test]
fn test_long_task_name_does_not_panic() -> Result<()> {
    let inventory_file = local_inventory();
    let playbook_file = fixture_file(
        "---\n- name: long title\n  hosts: localhost\n  tasks:\n    - name: this task name is intentionally longer than seventy two characters and must not panic\n      debug:\n        msg: hello\n",
    );
    let inventory = inventory::parse(inventory_file.path().to_str().unwrap())?;

    playbook::execute(playbook_file.path().to_str().unwrap(), &inventory)?;
    Ok(())
}

#[test]
fn test_builtin_fqcn_is_accepted() -> Result<()> {
    let inventory_file = local_inventory();
    let playbook_file = fixture_file(
        "---\n- name: fqcn\n  hosts: localhost\n  tasks:\n    - name: builtin debug\n      ansible.builtin.debug:\n        msg: hello\n",
    );
    let inventory = inventory::parse(inventory_file.path().to_str().unwrap())?;

    playbook::execute(playbook_file.path().to_str().unwrap(), &inventory)?;
    Ok(())
}

#[test]
fn test_check_mode_cli_does_not_modify_files() -> Result<()> {
    let inventory_file = local_inventory();
    let output_dir = TempDir::new()?;
    let marker = output_dir.path().join("cli-check-marker");
    let playbook_file = fixture_file(&format!(
        "---\n- name: cli check\n  hosts: localhost\n  tasks:\n    - name: must not run\n      command: touch {}\n",
        marker.display()
    ));

    let status = Command::new(env!("CARGO_BIN_EXE_rustsible"))
        .arg("playbook")
        .arg(playbook_file.path())
        .arg("-i")
        .arg(inventory_file.path())
        .arg("--check")
        .status()?;

    assert!(status.success());
    assert!(!marker.exists());
    Ok(())
}

#[test]
fn documented_subset_examples_remain_parseable() -> Result<()> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let inventory = inventory::Inventory::new();
    for relative_path in [
        "playbooks/test_all_modules.yml",
        "playbooks/test_command.yml",
        "playbooks/test_lineinfile.yml",
        "playbooks/test_service.yml",
        "playbooks/test_shell.yml",
        "playbooks/test_user.yml",
        "manual/local-smoke.yml",
        "manual/cleanup-local.yml",
    ] {
        let path = manifest_dir.join("examples").join(relative_path);
        let error = playbook::execute(path.to_str().unwrap(), &inventory).unwrap_err();
        let message = format!("{error:#}");
        assert!(
            message.contains("No hosts matched"),
            "documented example {relative_path} failed before host selection: {message}"
        );
    }
    Ok(())
}
