use anyhow::Result;
use std::time::{Duration, Instant};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;
mod support;

fn run(directory: &Path, book: &str, flags: &[&str]) -> Result<Output> {
    fs::write(directory.join("site.yml"), book)?;
    fs::write(
        directory.join("inventory"),
        "localhost ansible_connection=local\n",
    )?;
    Ok(Command::new(env!("CARGO_BIN_EXE_rustsible"))
        .arg("--worker-dir")
        .arg(support::worker_bundle())
        .arg("playbook")
        .arg(directory.join("site.yml"))
        .arg("-i")
        .arg(directory.join("inventory"))
        .args(flags)
        .output()?)
}
fn diagnostics(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn ad_hoc_background_and_poll_flags_return_queryable_jobs_after_controller_exit() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let inventory = directory.path().join("inventory");
    fs::write(
        &inventory,
        format!("localhost ansible_connection=local ansible_async_dir={root}/jobs\n"),
    )?;
    let execute = |module: &str, arguments: &str, flags: &[&str]| -> Result<Output> {
        Ok(Command::new(env!("CARGO_BIN_EXE_rustsible"))
            .arg("--worker-dir")
            .arg(support::worker_bundle())
            .arg("ad-hoc")
            .arg("localhost")
            .arg("-i")
            .arg(&inventory)
            .arg("-m")
            .arg(module)
            .arg("-a")
            .arg(arguments)
            .args(flags)
            .output()?)
    };
    let launched = execute("shell", &format!("while [ ! -f {root}/gate ]; do sleep 0.01; done; printf cli-complete > {root}/business"), &["-B", "10", "-P", "0"])?;
    assert!(launched.status.success(), "{}", diagnostics(&launched));
    let pattern = regex::Regex::new(r#""ansible_job_id"\s*:\s*"(j[a-f0-9]{64})""#)?;
    let text = String::from_utf8_lossy(&launched.stdout);
    let jid = pattern.captures(&text).expect("structured job ID")[1].to_owned();
    assert!(!directory.path().join("business").exists());
    fs::write(directory.path().join("gate"), "ready")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let finished = regex::Regex::new(r#""finished"\s*:\s*true"#)?;
    loop {
        let queried = execute("async_status", &format!("jid={jid}"), &[])?;
        assert!(queried.status.success(), "{}", diagnostics(&queried));
        if finished.is_match(&String::from_utf8_lossy(&queried.stdout)) {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        fs::read_to_string(directory.path().join("business"))?,
        "cli-complete"
    );
    assert!(
        execute("async_status", &format!("jid={jid} mode=cleanup"), &[])?
            .status
            .success()
    );
    assert_eq!(fs::read_dir(directory.path().join("jobs"))?.count(), 0);
    Ok(())
}

#[test]
fn playbook_background_query_until_finished_and_cleanup_keep_real_file_results() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let output = run(
        directory.path(),
        &format!(
            r#"
- hosts: localhost
  gather_facts: false
  vars:
    ansible_async_dir: {root}/jobs
  tasks:
    - shell: 'while [ ! -f {root}/gate ]; do sleep 0.01; done; printf complete > {root}/business; printf done'
      async: 10
      poll: 0
      register: launched
    - copy:
        content: ready
        dest: {root}/gate
    - async_status:
        jid: '{{{{ launched.ansible_job_id }}}}'
      register: observed
      until: observed is finished
      retries: 80
      delay: 0.02
    - copy:
        content: '{{{{ observed.stdout }}}}|{{{{ observed.rc }}}}'
        dest: {root}/registered
    - async_status:
        jid: '{{{{ launched.ansible_job_id }}}}'
        mode: cleanup
"#
        ),
        &[],
    )?;
    assert!(output.status.success(), "{}", diagnostics(&output));
    assert_eq!(
        fs::read_to_string(directory.path().join("business"))?,
        "complete"
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("registered"))?,
        "done|0"
    );
    assert_eq!(fs::read_dir(directory.path().join("jobs"))?.count(), 0);
    Ok(())
}

#[test]
fn positive_poll_waits_and_cleans_state_after_preserving_the_registered_result() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let output = run(
        directory.path(),
        &format!(
            r#"
- hosts: localhost
  gather_facts: false
  vars:
    ansible_async_dir: {root}/jobs
  tasks:
    - command:
        argv: [/bin/echo, polled]
      async: 5
      poll: 1
      register: completed
    - copy:
        content: '{{{{ completed.stdout }}}}|{{{{ "finished" if completed.finished else "running" }}}}'
        dest: {root}/registered
"#
        ),
        &[],
    )?;
    assert!(output.status.success(), "{}", diagnostics(&output));
    assert_eq!(
        fs::read_to_string(directory.path().join("registered"))?,
        "polled|finished"
    );
    assert_eq!(fs::read_dir(directory.path().join("jobs"))?.count(), 0);
    Ok(())
}

#[test]
fn async_check_never_starts_jobs_and_unsupported_payload_tasks_fail_before_business_mutation(
) -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let output = run(
        directory.path(),
        &format!(
            r#"
- hosts: localhost
  gather_facts: false
  vars:
    ansible_async_dir: {root}/jobs
  tasks:
    - command: touch {root}/business
      async: 5
      poll: 0
      check_mode: false
      register: prediction
    - debug:
        msg: skipped
      failed_when: prediction is not skipped
"#
        ),
        &["--check"],
    )?;
    assert!(output.status.success(), "{}", diagnostics(&output));
    assert!(!directory.path().join("business").exists());
    assert!(!directory.path().join("jobs").exists());
    let output = run(
        directory.path(),
        &format!(
            r#"
- hosts: localhost
  gather_facts: false
  vars:
    ansible_async_dir: {root}/jobs
  tasks:
    - copy:
        content: must-not-write
        dest: {root}/unsupported
      async: 5
      poll: 0
"#
        ),
        &[],
    )?;
    assert!(!output.status.success());
    assert!(!directory.path().join("unsupported").exists());
    assert!(!directory.path().join("jobs").exists());
    Ok(())
}

#[test]
fn no_log_is_retained_when_a_private_background_result_is_queried_by_a_later_task() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let output = run(
        directory.path(),
        &format!(
            r#"
- hosts: localhost
  gather_facts: false
  vars:
    ansible_async_dir: {root}/jobs
  tasks:
    - command: /bin/echo ASYNC_PRIVATE_SENTINEL
      async: 5
      poll: 0
      no_log: true
      register: private_job
    - async_status:
        jid: '{{{{ private_job.ansible_job_id }}}}'
      register: private_result
      until: private_result is finished
      retries: 80
      delay: 0.02
    - copy:
        content: '{{{{ private_result.stdout }}}}'
        dest: {root}/secret-business
      no_log: true
    - async_status:
        jid: '{{{{ private_job.ansible_job_id }}}}'
        mode: cleanup
"#
        ),
        &["-vvvv"],
    )?;
    assert!(output.status.success(), "{}", diagnostics(&output));
    assert_eq!(
        fs::read_to_string(directory.path().join("secret-business"))?,
        "ASYNC_PRIVATE_SENTINEL"
    );
    assert!(
        !diagnostics(&output).contains("ASYNC_PRIVATE_SENTINEL"),
        "{}",
        diagnostics(&output)
    );
    Ok(())
}
