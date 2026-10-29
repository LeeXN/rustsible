use anyhow::Result;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

mod support;

fn run(directory: &Path, playbook: &str, hosts: &str) -> Result<Output> {
    fs::write(directory.join("site.yml"), playbook)?;
    fs::write(directory.join("inventory"), hosts)?;
    Ok(Command::new(env!("CARGO_BIN_EXE_rustsible"))
        .arg("--worker-dir")
        .arg(support::worker_bundle())
        .arg("playbook")
        .arg(directory.join("site.yml"))
        .arg("-i")
        .arg(directory.join("inventory"))
        .args(["--forks", "4"])
        .output()?)
}
fn diagnostics(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}
const THREE: &str =
    "a ansible_connection=local\nb ansible_connection=local\nc ansible_connection=local\n";
const FOUR: &str = "a ansible_connection=local\nb ansible_connection=local\nc ansible_connection=local\nd ansible_connection=local\n";

#[test]
fn delegation_uses_target_connections_keeps_logical_arguments_and_assigns_facts_explicitly(
) -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let output = run(directory.path(), &format!(r#"
- hosts: origin
  gather_facts: false
  tasks:
    - copy:
        content: '{{{{ marker }}}}|{{{{ inventory_hostname }}}}'
        dest: {root}/delegated
      delegate_to: target
      register: deployed
    - setup:
        filter: ansible_system
      delegate_to: target
      delegate_facts: true
    - set_fact:
        assigned: '{{{{ inventory_hostname }}}}'
      delegate_to: target
      delegate_facts: true
    - set_fact:
        default_assigned: origin
      delegate_to: target
    - copy:
        content: '{{{{ hostvars.target.assigned }}}}|{{{{ hostvars.target.ansible_facts.system }}}}|{{{{ deployed.delegated_to }}}}|{{{{ default_assigned }}}}|{{{{ hostvars.origin.ansible_facts | default({{}}) | length }}}}'
        dest: {root}/facts
      delegate_to: localhost
"#), "origin ansible_connection=ssh ansible_host=unreachable.invalid marker=origin\ntarget ansible_connection=local marker=target\n")?;
    assert!(output.status.success(), "{}", diagnostics(&output));
    assert_eq!(
        fs::read_to_string(directory.path().join("delegated"))?,
        "origin|origin"
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("facts"))?,
        "origin|Linux|target|origin|0"
    );
    Ok(())
}

#[test]
fn loop_delegated_facts_keep_each_item_target_and_implicit_local_connection() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let output = run(directory.path(), &format!(r#"
- hosts: origin
  gather_facts: false
  tasks:
    - set_fact:
        assigned: '{{{{ item }}}}|{{{{ inventory_hostname }}}}'
        iteration_count: '{{{{ (iteration_count | default(0)) + 1 }}}}'
      delegate_to: '{{{{ item }}}}'
      delegate_facts: true
      loop: [target, localhost]
      register: delegated_facts
    - copy:
        content: '{{{{ hostvars.target.assigned }}}}|{{{{ hostvars.localhost.assigned }}}}|{{{{ delegated_facts.results | length }}}}|{{{{ hostvars.target.iteration_count }}}}|{{{{ hostvars.localhost.iteration_count }}}}'
        dest: {root}/facts
      delegate_to: localhost
"#), "origin ansible_connection=ssh ansible_host=unreachable.invalid\ntarget ansible_connection=local\n")?;
    assert!(output.status.success(), "{}", diagnostics(&output));
    assert_eq!(
        fs::read_to_string(directory.path().join("facts"))?,
        "target|origin|localhost|origin|2|1|1"
    );
    Ok(())
}

#[test]
fn play_run_once_keeps_gathering_per_host_and_counts_only_actual_executions() -> Result<()> {
    let directory = TempDir::new()?;
    let output = run(
        directory.path(),
        r#"
- hosts: all
  run_once: true
  tasks:
    - debug:
        msg: first
      register: leader
    - debug:
        msg: '{{ leader.msg }}'
      run_once: false
"#,
        "a ansible_connection=local\nb ansible_connection=local\n",
    )?;
    assert!(output.status.success(), "{}", diagnostics(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("a                              : ok=3 changed=0"),
        "{stdout}"
    );
    assert!(
        stdout.contains("b                              : ok=2 changed=0"),
        "{stdout}"
    );
    Ok(())
}

#[test]
fn previously_failed_hosts_cannot_block_a_later_healthy_serial_batch() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let output = run(
        directory.path(),
        &format!(
            r#"
- hosts: all
  gather_facts: false
  tasks:
    - command: /bin/false
      when: inventory_hostname == 'a'
- hosts: all
  gather_facts: false
  serial: 1
  tasks:
    - command: touch {root}/later-{{{{ inventory_hostname }}}}
"#
        ),
        THREE,
    )?;
    assert!(!output.status.success());
    assert!(!directory.path().join("later-a").exists());
    assert!(directory.path().join("later-b").exists());
    assert!(directory.path().join("later-c").exists());
    Ok(())
}

#[test]
fn rolling_batches_share_run_once_results_and_flush_handlers_before_next_batch() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let output = run(
        directory.path(),
        &format!(
            r#"
- hosts: all
  gather_facts: false
  serial: [2, 1]
  tasks:
    - shell: test -f {root}/handled-a
      when: inventory_hostname == 'c'
    - command: /bin/echo '{{{{ inventory_hostname }}}}'
      run_once: true
      register: leader
      notify: finish
    - copy:
        content: '{{{{ ansible_play_batch | join(",") }}}}|{{{{ leader.stdout }}}}'
        dest: {root}/batch-{{{{ inventory_hostname }}}}
  handlers:
    - name: finish
      copy:
        content: handled
        dest: {root}/handled-{{{{ inventory_hostname }}}}
"#
        ),
        THREE,
    )?;
    assert!(output.status.success(), "{}", diagnostics(&output));
    for (host, expected) in [("a", "a,b|a"), ("b", "a,b|a"), ("c", "c|c")] {
        assert_eq!(
            fs::read_to_string(directory.path().join(format!("batch-{host}")))?,
            expected
        );
        assert_eq!(
            directory.path().join(format!("handled-{host}")).exists(),
            host != "b"
        );
    }
    Ok(())
}

#[test]
fn throttle_caps_business_operations_inside_a_larger_forks_pool() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let output = run(
        directory.path(),
        &format!(
            r#"
- hosts: all
  gather_facts: false
  throttle: 1
  tasks:
    - shell: 'mkdir {root}/lock && sleep 0.03 && printf "%s\n" "{{{{ inventory_hostname }}}}" >> {root}/order && rmdir {root}/lock'
"#
        ),
        THREE,
    )?;
    assert!(output.status.success(), "{}", diagnostics(&output));
    let order = fs::read_to_string(directory.path().join("order"))?;
    let mut hosts = order.lines().collect::<Vec<_>>();
    hosts.sort();
    assert_eq!(hosts, vec!["a", "b", "c"]);
    assert!(!directory.path().join("lock").exists());
    Ok(())
}

#[test]
fn failure_percentage_is_strict_and_only_exceeded_limits_stop_later_batches() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    for percentage in [49, 50] {
        let output = run(
            directory.path(),
            &format!(
                r#"
- hosts: all
  gather_facts: false
  serial: 2
  max_fail_percentage: {percentage}
  tasks:
    - command: /bin/false
      when: inventory_hostname == 'a'
    - command: touch {root}/{percentage}-{{{{ inventory_hostname }}}}
"#
            ),
            FOUR,
        )?;
        assert!(!output.status.success());
        assert!(!directory.path().join(format!("{percentage}-a")).exists());
        for host in ["b", "c", "d"] {
            assert_eq!(
                directory
                    .path()
                    .join(format!("{percentage}-{host}"))
                    .exists(),
                percentage == 50,
                "{percentage}/{host}: {}",
                diagnostics(&output)
            );
        }
    }
    Ok(())
}

#[test]
fn fatal_errors_allow_rescue_and_always_but_stop_following_tasks_and_plays() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    for rescue in [true, false] {
        let rescue_tasks = if rescue {
            "      rescue:\n        - command: /bin/true\n"
        } else {
            ""
        };
        let output = run(
            directory.path(),
            &format!(
                r#"
- hosts: all
  gather_facts: false
  any_errors_fatal: true
  tasks:
    - block:
        - command: /bin/false
          when: inventory_hostname == 'a'
{rescue_tasks}      always:
        - command: touch {root}/{rescue}-always-{{{{ inventory_hostname }}}}
    - command: touch {root}/{rescue}-later-{{{{ inventory_hostname }}}}
- hosts: all
  gather_facts: false
  tasks:
    - command: touch {root}/{rescue}-play-{{{{ inventory_hostname }}}}
"#
            ),
            "a ansible_connection=local\nb ansible_connection=local\n",
        )?;
        assert!(!output.status.success(), "{}", diagnostics(&output));
        for host in ["a", "b"] {
            assert!(directory
                .path()
                .join(format!("{rescue}-always-{host}"))
                .exists());
            assert_eq!(
                directory
                    .path()
                    .join(format!("{rescue}-later-{host}"))
                    .exists(),
                rescue
            );
            assert_eq!(
                directory
                    .path()
                    .join(format!("{rescue}-play-{host}"))
                    .exists(),
                rescue
            );
        }
    }
    Ok(())
}
