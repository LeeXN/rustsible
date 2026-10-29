use anyhow::Result;
use std::{fs, process::Command};
use tempfile::TempDir;
mod support;

#[test]
fn assert_fail_share_conditions_recovery_and_check_without_connecting() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    fs::write(
        directory.path().join("inventory"),
        "origin ansible_connection=ssh ansible_host=unreachable.invalid\n",
    )?;
    fs::write(
        directory.path().join("site.yml"),
        format!(
            r#"
- hosts: all
  gather_facts: false
  vars:
    amount: 3
  tasks:
    - assert:
        that: [amount == 3, true]
        success_msg: accepted
      register: passed
    - block:
        - assert:
            that: amount < 1
            msg: expected assertion
      rescue:
        - assert:
            that:
              - ansible_failed_result.evaluated_to == false
              - ansible_failed_result.assertion == 'amount < 1'
    - block:
        - fail:
            msg: expected fail
      rescue:
        - assert:
            that: ansible_failed_result.msg == 'expected fail'
    - set_fact:
        ansible_connection: local
    - copy:
        content: '{{{{ passed.msg }}}}'
        dest: {root}/result
"#
        ),
    )?;
    let run = |check: bool| -> Result<std::process::Output> {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rustsible"));
        command
            .arg("--worker-dir")
            .arg(support::worker_bundle())
            .arg("playbook")
            .arg(directory.path().join("site.yml"))
            .arg("-i")
            .arg(directory.path().join("inventory"));
        if check {
            command.arg("--check");
        }
        Ok(command.output()?)
    };
    let predicted = run(true)?;
    assert!(
        predicted.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&predicted.stdout),
        String::from_utf8_lossy(&predicted.stderr)
    );
    assert!(!directory.path().join("result").exists());
    let actual = run(false)?;
    assert!(
        actual.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&actual.stdout),
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("result"))?,
        "accepted"
    );
    Ok(())
}

#[test]
fn assertions_require_booleans_keep_runtime_data_literal_and_honor_no_log() -> Result<()> {
    let directory = TempDir::new()?;
    fs::write(
        directory.path().join("inventory"),
        "origin ansible_connection=ssh ansible_host=unreachable.invalid\n",
    )?;
    for expression in ["'false'", "'{{ injected }}'"] {
        fs::write(
            directory.path().join("site.yml"),
            format!(
                r#"
- hosts: all
  gather_facts: false
  no_log: true
  tasks:
    - assert:
        that: "{expression}"
        fail_msg: ASSERT_PRIVATE_SENTINEL
"#
            ),
        )?;
        let output = Command::new(env!("CARGO_BIN_EXE_rustsible"))
            .arg("playbook")
            .arg(directory.path().join("site.yml"))
            .arg("-i")
            .arg(directory.path().join("inventory"))
            .output()?;
        assert!(!output.status.success());
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!text.contains("ASSERT_PRIVATE_SENTINEL"), "{text}");
        assert!(text.contains("output censored"), "{text}");
    }
    Ok(())
}
