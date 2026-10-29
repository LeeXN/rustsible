use anyhow::Result;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

mod support;

fn run(directory: &Path, playbook: &str, hosts: &str) -> Result<Output> {
    run_with_flags(directory, playbook, hosts, &[])
}
fn run_with_flags(directory: &Path, playbook: &str, hosts: &str, flags: &[&str]) -> Result<Output> {
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
const THREE: &str =
    "a ansible_connection=local\nb ansible_connection=local\nc ansible_connection=local\n";
const FOUR: &str = "a ansible_connection=local\nb ansible_connection=local\nc ansible_connection=local\nd ansible_connection=local\n";

#[test]
fn free_hosts_advance_without_a_task_barrier_and_observe_published_host_facts() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let output = run(
        directory.path(),
        &format!(
            r#"
- hosts: all
  gather_facts: false
  strategy: free
  tasks:
    - shell: 'if [ "{{{{ inventory_hostname }}}}" = a ]; then n=0; while [ ! -f {root}/signal ]; do n=$((n+1)); [ "$n" -le 1000 ] || exit 41; sleep 0.01; done; fi'
      timeout: 15
      delegate_to: a
    - set_fact:
        published: from-b
      when: inventory_hostname == 'b'
    - copy:
        content: ready
        dest: {root}/signal
      when: inventory_hostname == 'b'
      delegate_to: a
      notify: completed
    - copy:
        content: '{{{{ hostvars.b.published }}}}'
        dest: {root}/observed-{{{{ inventory_hostname }}}}
  handlers:
    - name: completed
      copy:
        content: handled
        dest: {root}/handled-{{{{ inventory_hostname }}}}
"#
        ),
        "a ansible_connection=local\nb ansible_connection=local\n",
    )?;
    assert!(output.status.success(), "{}", diagnostics(&output));
    for host in ["a", "b"] {
        assert_eq!(
            fs::read_to_string(directory.path().join(format!("observed-{host}")))?,
            "from-b"
        );
    }
    assert!(directory.path().join("handled-b").exists());
    assert!(!directory.path().join("handled-a").exists());
    Ok(())
}

#[test]
fn free_throttle_is_shared_across_independent_host_controllers() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let output = run(
        directory.path(),
        &format!(
            r#"
- hosts: all
  gather_facts: false
  strategy: free
  tasks:
    - shell: 'mkdir {root}/lock && sleep 0.03 && printf "%s\n" "{{{{ inventory_hostname }}}}" >> {root}/order && rmdir {root}/lock'
      throttle: 1
"#
        ),
        FOUR,
    )?;
    assert!(output.status.success(), "{}", diagnostics(&output));
    let contents = fs::read_to_string(directory.path().join("order"))?;
    let mut hosts = contents.lines().collect::<Vec<_>>();
    hosts.sort();
    assert_eq!(hosts, vec!["a", "b", "c", "d"]);
    assert!(!directory.path().join("lock").exists());
    Ok(())
}

#[test]
fn free_check_cannot_be_disabled_by_a_task_and_unsupported_controls_fail_before_mutation(
) -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let output = run_with_flags(
        directory.path(),
        &format!(
            r#"
- hosts: all
  gather_facts: false
  strategy: free
  tasks:
    - copy:
        content: must-not-write
        dest: {root}/check-{{{{ inventory_hostname }}}}
      check_mode: false
"#
        ),
        THREE,
        &["--check"],
    )?;
    assert!(output.status.success(), "{}", diagnostics(&output));
    for host in ["a", "b", "c"] {
        assert!(!directory.path().join(format!("check-{host}")).exists());
    }
    for control in [
        "run_once: true",
        "any_errors_fatal: true",
        "max_fail_percentage: 50",
    ] {
        let output = run(
            directory.path(),
            &format!(
                r#"
- hosts: all
  gather_facts: false
  strategy: free
  {control}
  tasks:
    - command: touch {root}/mutated
"#
            ),
            THREE,
        )?;
        assert!(!output.status.success());
        assert!(!directory.path().join("mutated").exists());
        assert!(
            diagnostics(&output).contains("free strategy cannot"),
            "{}",
            diagnostics(&output)
        );
    }
    Ok(())
}

#[test]
fn free_dynamic_public_roles_publish_to_other_hosts_without_leaking_into_later_plays() -> Result<()>
{
    let directory = TempDir::new()?;
    let root = directory.path().display();
    fs::create_dir_all(directory.path().join("roles/demo/vars"))?;
    fs::write(
        directory.path().join("roles/demo/vars/main.yml"),
        "published_role: from-role\n",
    )?;
    let output = run(
        directory.path(),
        &format!(
            r#"
- hosts: all
  gather_facts: false
  strategy: free
  tasks:
    - include_role:
        name: demo
        public: true
      when: inventory_hostname == 'b'
    - shell: 'if [ "{{{{ inventory_hostname }}}}" = a ]; then n=0; while [ ! -f {root}/role-ready ]; do n=$((n+1)); [ "$n" -le 1000 ] || exit 41; sleep 0.01; done; fi'
      timeout: 15
    - copy:
        content: ready
        dest: {root}/role-ready
      when: inventory_hostname == 'b'
    - copy:
        content: '{{{{ published_role }}}}'
        dest: {root}/role-{{{{ inventory_hostname }}}}
- hosts: all
  gather_facts: false
  tasks:
    - copy:
        content: '{{{{ published_role | default("private") }}}}'
        dest: {root}/later-{{{{ inventory_hostname }}}}
"#
        ),
        "a ansible_connection=local\nb ansible_connection=local\n",
    )?;
    assert!(output.status.success(), "{}", diagnostics(&output));
    for host in ["a", "b"] {
        assert_eq!(
            fs::read_to_string(directory.path().join(format!("role-{host}")))?,
            "from-role"
        );
        assert_eq!(
            fs::read_to_string(directory.path().join(format!("later-{host}")))?,
            "private"
        );
    }
    Ok(())
}

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
fn previous_failures_keep_original_percentage_partitions_and_failure_denominators() -> Result<()> {
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
  serial: '50%'
  tasks:
    - copy:
        content: '{{{{ ansible_play_batch | join(",") }}}}'
        dest: {root}/batch-{{{{ inventory_hostname }}}}
- hosts: all
  gather_facts: false
  serial: '50%'
  max_fail_percentage: 50
  tasks:
    - command: /bin/false
      when: inventory_hostname == 'b'
    - command: touch {root}/survived-{{{{ inventory_hostname }}}}
"#
        ),
        FOUR,
    )?;
    assert!(!output.status.success());
    for (host, batch) in [("b", "b"), ("c", "c,d"), ("d", "c,d")] {
        assert_eq!(
            fs::read_to_string(directory.path().join(format!("batch-{host}")))?,
            batch,
            "{}",
            diagnostics(&output)
        );
    }
    assert!(!directory.path().join("survived-b").exists());
    for host in ["c", "d"] {
        assert!(directory.path().join(format!("survived-{host}")).exists());
    }
    Ok(())
}

#[test]
fn serial_reads_static_vars_files_and_literal_counts_do_not_force_early_fact_paths() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    fs::write(directory.path().join("deployment.yml"), "batch_size: 2\n")?;
    fs::write(
        directory
            .path()
            .join(format!("{}.yml", std::env::consts::ARCH)),
        "batch_size: 2\n",
    )?;
    for (name, serial, file, gather) in [
        ("template", "'{{ batch_size }}'", "deployment.yml", false),
        ("literal", "2", "{{ ansible_architecture }}.yml", true),
    ] {
        let output = run(
            directory.path(),
            &format!(
                r#"
- hosts: all
  gather_facts: {gather}
  gather_subset: [architecture]
  serial: {serial}
  vars_files: ['{file}']
  tasks:
    - copy:
        content: '{{{{ ansible_play_batch | join(",") }}}}'
        dest: {root}/{name}-{{{{ inventory_hostname }}}}
"#
            ),
            THREE,
        )?;
        assert!(output.status.success(), "{}", diagnostics(&output));
        for (host, batch) in [("a", "a,b"), ("b", "a,b"), ("c", "c")] {
            assert_eq!(
                fs::read_to_string(directory.path().join(format!("{name}-{host}")))?,
                batch
            );
        }
    }
    Ok(())
}

#[test]
fn serial_planning_errors_respect_play_no_log_even_with_verbose_logging() -> Result<()> {
    let directory = TempDir::new()?;
    fs::write(
        directory.path().join("SERIAL_PRIVATE_SENTINEL.yml"),
        "[not, a, mapping]\n",
    )?;
    let output = run_with_flags(
        directory.path(),
        r#"
- hosts: all
  gather_facts: false
  no_log: true
  serial: '{{ batch_size }}'
  vars_files: ['{{ private_file }}']
  vars:
    private_file: SERIAL_PRIVATE_SENTINEL.yml
  tasks:
    - debug: {msg: unreachable}
"#,
        THREE,
        &["-vvvv"],
    )?;
    assert!(!output.status.success());
    let text = diagnostics(&output);
    assert!(text.contains("output censored"), "{text}");
    assert!(!text.contains("SERIAL_PRIVATE_SENTINEL"), "{text}");
    Ok(())
}

#[test]
fn free_dynamic_imports_validate_controls_only_when_selected_before_running_children() -> Result<()>
{
    let directory = TempDir::new()?;
    let root = directory.path().display();
    fs::write(
        directory.path().join("selected.yml"),
        format!("- command: touch {root}/must-not-run\n  run_once: true\n"),
    )?;
    for selected in [false, true] {
        let output = run(
            directory.path(),
            &format!(
                r#"
- hosts: all
  gather_facts: false
  strategy: free
  tasks:
    - include_tasks: selected.yml
      when: {selected}
    - copy:
        content: continued
        dest: {root}/{selected}-{{{{ inventory_hostname }}}}
"#
            ),
            THREE,
        )?;
        assert_eq!(
            output.status.success(),
            !selected,
            "{}",
            diagnostics(&output)
        );
        assert!(!directory.path().join("must-not-run").exists());
        for host in ["a", "b", "c"] {
            assert_eq!(
                directory.path().join(format!("{selected}-{host}")).exists(),
                !selected
            );
        }
    }
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
                rescue && host == "a"
            );
        }
    }
    Ok(())
}
