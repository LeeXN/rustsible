use anyhow::Result;
use rustsible::inventory;
use rustsible::playbook;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use tempfile::{NamedTempFile, TempDir};

mod support;
fn controller() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rustsible"));
    command.arg("--worker-dir").arg(support::worker_bundle());
    command
}
fn run_playbook(file: &str, inventory: &inventory::Inventory) -> Result<()> {
    playbook::execute_with_options(
        file,
        inventory,
        &playbook::ExecutionOptions {
            worker_dir: Some(support::worker_bundle()),
            ..playbook::ExecutionOptions::default()
        },
    )
}

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
fn inventory_playbook_variable_files_and_extra_vars_keep_layers_and_source_paths() -> Result<()> {
    let directory = TempDir::new()?;
    let inventory_dir = directory.path().join("inventory");
    let play_dir = directory.path().join("playbook");
    let elsewhere = directory.path().join("elsewhere");
    for base in [&inventory_dir, &play_dir] {
        std::fs::create_dir_all(base.join("group_vars"))?;
        std::fs::create_dir_all(base.join("host_vars"))?;
    }
    std::fs::create_dir(&elsewhere)?;
    std::fs::write(
        inventory_dir.join("hosts"),
        "[web]\nnode ansible_host=unreachable.invalid ansible_connection=ssh priority=inventory\n",
    )?;
    std::fs::write(
        inventory_dir.join("group_vars/all.yml"),
        "group_value: inventory\npriority: inventorygroup",
    )?;
    std::fs::write(
        inventory_dir.join("host_vars/node.yml"),
        "priority: inventoryhost",
    )?;
    std::fs::write(play_dir.join("group_vars/all.yml"), "group_value: playbook")?;
    std::fs::write(play_dir.join("host_vars/node.yml"), "host_value: playbook")?;
    std::fs::write(
        play_dir.join("choice.yml"),
        "priority: varsfile\ntyped: false\nfrom_file: [one, two]\nansible_connection: local\nansible_ssh_host: localhost",
    )?;
    std::fs::write(
        directory.path().join("extra.yml"),
        "selector: choice\npriority: extrafile\nnumber: 7",
    )?;
    std::fs::write(
        play_dir.join("site.yml"),
        format!(
            r#"
- hosts: all
  gather_facts: true
  vars:
    priority: play
    ansible_host: unreachable.invalid
  vars_files:
    - [missing.yml, '{{{{ selector }}}}.yml']
  tasks:
    - command: /bin/true
      register: priority
      retries: 1
      delay: 0
      until: priority.rc == 0
    - copy:
        content: '{{{{ priority }}}}|{{{{ group_value }}}}|{{{{ host_value }}}}|{{{{ typed }}}}|{{{{ number }}}}|{{{{ from_file | join(",") }}}}'
        dest: {}/result
      vars:
        priority: task
        typed: true
"#,
            directory.path().display()
        ),
    )?;
    let output = controller()
        .current_dir(&elsewhere)
        .args([
            "playbook",
            play_dir.join("site.yml").to_str().unwrap(),
            "-i",
            inventory_dir.join("hosts").to_str().unwrap(),
            "-e",
            &format!("@{}", directory.path().join("extra.yml").display()),
            "-e",
            "priority=extra",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("result"))?,
        "extra|playbook|playbook|True|7|one,two"
    );
    Ok(())
}

#[test]
fn include_vars_and_set_fact_keep_lifetimes_types_and_loop_accumulation() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    std::fs::write(
        directory.path().join("included.yml"),
        "priority: included\ndelayed: '{{ later }}'\n",
    )?;
    std::fs::create_dir(directory.path().join("variables"))?;
    std::fs::write(directory.path().join("variables/a.yml"), "value: first")?;
    std::fs::write(
        directory.path().join("variables/z.json"),
        r#"{"value":"last","typed":true}"#,
    )?;
    let book = directory.path().join("site.yml");
    std::fs::write(
        &book,
        format!(
            r#"
- hosts: localhost
  gather_facts: false
  vars:
    priority: play
  tasks:
    - include_vars: included.yml
      vars:
        priority: task
    - set_fact:
        later: target
        items: []
    - copy:
        content: '{{{{ priority }}}}|{{{{ delayed }}}}'
        dest: {root}/included
      vars:
        priority: task
    - set_fact:
        items: '{{{{ items + [item] }}}}'
      loop: [1, 2]
    - set_fact:
        priority: fact
    - include_vars:
        dir: variables
        name: namespaced
    - set_fact:
        qualified: []
    - set_fact:
        qualified: '{{{{ hostvars[inventory_hostname].qualified + [item] }}}}'
      loop: [1, 2]
- hosts: localhost
  gather_facts: false
  vars:
    priority: secondplay
  tasks:
    - copy:
        content: '{{{{ priority }}}}|{{{{ delayed }}}}|{{{{ items | join(",") }}}}|{{{{ namespaced.value }}}}|{{{{ namespaced.typed }}}}|{{{{ qualified | join(",") }}}}'
        dest: {root}/result
"#
        ),
    )?;
    let inventory = local_inventory();
    let output = controller()
        .args([
            "playbook",
            book.to_str().unwrap(),
            "-i",
            inventory.path().to_str().unwrap(),
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("included"))?,
        "included|target"
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("result"))?,
        "fact|target|1,2|last|True|2"
    );
    Ok(())
}

#[test]
fn extra_vars_render_ad_hoc_arguments_and_debug_using_the_shared_template_path() -> Result<()> {
    let inventory = local_inventory();
    let output = controller()
        .args([
            "ad-hoc",
            "all",
            "-i",
            inventory.path().to_str().unwrap(),
            "-m",
            "command",
            "-a",
            r#"{"argv":["/usr/bin/printf","%s|%s","{{ text }}","{{ items | join(',') }}"]}"#,
            "-e",
            r#"{"text":"two words; literal","items":[1,2]}"#,
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("two words; literal|1,2"));
    let output = controller()
        .args([
            "ad-hoc",
            "all",
            "-i",
            inventory.path().to_str().unwrap(),
            "-m",
            "debug",
            "-a",
            "var=structured",
            "-e",
            r#"{"structured":{"enabled":true,"items":[1,2]}}"#,
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("enabled") && text.contains("items"), "{text}");
    Ok(())
}

#[test]
fn hostvars_groups_and_source_paths_follow_host_scope_between_tasks() -> Result<()> {
    let directory = TempDir::new()?;
    let inventory = directory.path().join("inventory.yml");
    std::fs::write(
        &inventory,
        r#"
all:
  vars:
    ansible_connection: local
    ansible_host: localhost
  hosts:
    outside:
      token: untouched
  children:
    parent:
      children:
        web:
          hosts:
            one:
              linked: '{{ hostvars.two.token }}'
            two:
              token: inventory
"#,
    )?;
    let book = directory.path().join("site.yml");
    std::fs::write(
        &book,
        format!(
            r#"
- hosts: web
  gather_facts: false
  vars:
    play_only: invisible
  tasks:
    - set_fact:
        token: '{{{{ inventory_hostname }}}}-fact'
    - copy:
        content: '{{{{ hostvars.two.token }}}}|{{{{ hostvars.one.linked }}}}|{{{{ hostvars.outside.token }}}}|{{{{ groups.web | join(",") }}}}|{{{{ group_names | join(",") }}}}|{{{{ hostvars.one.play_only | default("absent") }}}}|{{{{ inventory_dir }}}}|{{{{ playbook_dir }}}}'
        dest: {}/{{{{ inventory_hostname }}}}
"#,
            directory.path().display()
        ),
    )?;
    let output = controller()
        .args([
            "playbook",
            book.to_str().unwrap(),
            "-i",
            inventory.to_str().unwrap(),
            "-e",
            "inventory_hostname=spoof",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for name in ["one", "two"] {
        assert_eq!(
            std::fs::read_to_string(directory.path().join(name))?,
            format!(
                "two-fact|two-fact|untouched|one,two|parent,web|absent|{}|{}",
                directory.path().display(),
                directory.path().display()
            )
        );
    }
    assert!(!directory.path().join("spoof").exists());
    let output = controller().args(["ad-hoc", "one", "-i", inventory.to_str().unwrap(), "-m", "debug", "-a", r#"{"msg":"{{ hostvars.two.token }}|{{ groups.web | join(',') }}|{{ group_names | join(',') }}"}"#]).output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("inventory|one,two|parent,web"));
    Ok(())
}

#[test]
fn configuration_selection_and_cli_overrides_are_visible_in_real_runs() -> Result<()> {
    let directory = TempDir::new()?;
    let project = directory.path().join("project");
    let environment = directory.path().join("environment");
    let explicit = directory.path().join("explicit");
    let home = directory.path().join("home");
    for (base, identity) in [
        (&project, "project"),
        (&environment, "environment"),
        (&explicit, "explicit"),
        (&home, "home"),
    ] {
        std::fs::create_dir(base)?;
        std::fs::write(
            base.join("hosts"),
            format!("node ansible_connection=local ansible_host=localhost identity={identity}\n"),
        )?;
        std::fs::write(
            base.join(if identity == "home" {
                ".ansible.cfg"
            } else {
                "ansible.cfg"
            }),
            "[defaults]\ninventory=hosts\nforks=2\nworker_dir=missing-workers\n",
        )?;
    }
    let book = project.join("site.yml");
    std::fs::write(&book, "- hosts: all\n  gather_facts: false\n  tasks:\n    - copy:\n        content: '{{ identity }}|{{ ansible_config_file }}'\n        dest: '{{ output }}'\n")?;
    for (index, choice) in [
        "project",
        "environment",
        "explicit",
        "cli-inventory",
        "home",
    ]
    .iter()
    .enumerate()
    {
        if *choice == "home" {
            std::fs::remove_file(project.join("ansible.cfg"))?;
        }
        let output_file = directory.path().join(format!("result-{index}"));
        let mut command = controller();
        command
            .current_dir(&project)
            .env("HOME", &home)
            .env_remove("ANSIBLE_CONFIG")
            .env_remove("ANSIBLE_INVENTORY")
            .env_remove("ANSIBLE_FORKS");
        if matches!(*choice, "environment" | "explicit" | "cli-inventory") {
            command.env("ANSIBLE_CONFIG", environment.join("ansible.cfg"));
        }
        if matches!(*choice, "explicit" | "cli-inventory") {
            command.arg("--config").arg(explicit.join("ansible.cfg"));
        }
        command
            .arg("playbook")
            .arg(&book)
            .arg("-e")
            .arg(format!("output={}", output_file.display()))
            .args(["--forks", "1"]);
        if *choice == "cli-inventory" {
            command.arg("-i").arg(project.join("hosts"));
        }
        let output = command.output()?;
        assert!(
            output.status.success(),
            "{choice}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let identity = if *choice == "cli-inventory" {
            "project"
        } else {
            choice
        };
        let source = match *choice {
            "project" => project.join("ansible.cfg"),
            "environment" => environment.join("ansible.cfg"),
            "home" => home.join(".ansible.cfg"),
            _ => explicit.join("ansible.cfg"),
        };
        assert_eq!(
            std::fs::read_to_string(output_file)?,
            format!("{identity}|{}", source.display())
        );
    }
    Ok(())
}

#[test]
fn configuration_become_defaults_stay_below_play_task_and_variable_settings() -> Result<()> {
    let directory = TempDir::new()?;
    std::fs::write(
        directory.path().join("hosts"),
        "node ansible_connection=local ansible_host=localhost\n",
    )?;
    std::fs::write(
        directory.path().join("ansible.cfg"),
        "[defaults]\ninventory=hosts\n[privilege_escalation]\nbecome=false\nbecome_user=root\n",
    )?;
    let book = directory.path().join("site.yml");
    std::fs::write(
        &book,
        r#"
- hosts: all
  gather_facts: false
  become: true
  become_user: app
  tasks:
    - debug:
        msg: 'play={{ ansible_become }}|{{ ansible_become_user }}'
    - debug:
        msg: 'task={{ ansible_become }}|{{ ansible_become_user }}'
      become: false
      become_user: nobody
    - debug:
        msg: 'variable={{ ansible_become }}|{{ ansible_become_user }}'
      vars:
        ansible_become: false
        ansible_become_user: variable-user
"#,
    )?;
    let output = controller()
        .arg("--config")
        .arg(directory.path().join("ansible.cfg"))
        .arg("playbook")
        .arg(&book)
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "play=True|app",
        "task=False|nobody",
        "variable=False|variable-user",
    ] {
        assert!(text.contains(expected), "{text}");
    }
    Ok(())
}

#[test]
fn task_attempts_conditions_environment_and_loop_results_follow_real_execution() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let file = fixture_file(&format!(
        r#"
- hosts: localhost
  gather_facts: false
  environment:
    RUSTSIBLE_TEST_LEVEL: play
    RUSTSIBLE_TEST_INHERITED: inherited
  tasks:
    - shell: 'printf "%s|%s" "$RUSTSIBLE_TEST_LEVEL" "$RUSTSIBLE_TEST_INHERITED"'
      environment:
        RUSTSIBLE_TEST_LEVEL: task
      register: environment_result
    - copy:
        content: '{{{{ environment_result.stdout }}}}'
        dest: {root}/environment
    - shell: 'n=$(cat {root}/count-{{{{ item }}}} 2>/dev/null || echo 0); n=$((n+1)); printf %s "$n" > {root}/count-{{{{ item }}}}; printf %s "$n"; test "$n" -ge 3'
      register: step
      loop: [one, two]
      retries: 3
      delay: 0
      until: step.rc == 0
      changed_when: step.stdout == '3'
      notify: mark
    - copy:
        content: '{{{{ step.results[0].attempts }}}}|{{{{ step.results[1].attempts }}}}|{{{{ step.results[0].item }}}}|{{{{ step.results[1].item }}}}'
        dest: {root}/attempts
    - command: /bin/false
      failed_when: false
      changed_when: false
      register: accepted
      notify: suppressed
    - copy:
        content: '{{{{ accepted.rc }}}}|{{{{ accepted.failed }}}}|{{{{ accepted.changed }}}}'
        dest: {root}/accepted
    - shell: 'printf x >> {root}/exhausted'
      register: exhausted
      retries: 1
      delay: 0
      until: false
      ignore_errors: true
    - copy:
        content: '{{{{ exhausted.attempts }}}}|{{{{ exhausted.failed }}}}'
        dest: {root}/exhaustion
    - shell: 'test "$RUSTSIBLE_TEST_LEVEL" = play'
      changed_when: false
    - shell: 'n=$(cat {root}/retry-success 2>/dev/null || echo 0); n=$((n+1)); printf %s "$n" > {root}/retry-success; test "$n" -ge 2'
      retries: 2
      delay: 0
      until: []
      register: retry_success
    - copy:
        content: '{{{{ retry_success.attempts | string }}}}'
        dest: {root}/retry-attempts
    - debug:
        msg: safe
      changed_when: []
      failed_when: []
      register: empty_conditions
    - copy:
        content: '{{{{ empty_conditions.changed }}}}|{{{{ empty_conditions.failed }}}}'
        dest: {root}/empty-conditions
  handlers:
    - name: mark
      shell: 'printf x >> {root}/handler'
    - name: suppressed
      shell: 'touch {root}/suppressed-handler'
"#
    ));
    let inventory = local_inventory();
    let output = controller()
        .args([
            "playbook",
            file.path().to_str().unwrap(),
            "-i",
            inventory.path().to_str().unwrap(),
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for (name, expected) in [
        ("environment", "task|inherited"),
        ("attempts", "3|3|one|two"),
        ("accepted", "1|False|False"),
        ("exhausted", "xx"),
        ("exhaustion", "1|True"),
        ("handler", "x"),
        ("retry-success", "2"),
        ("retry-attempts", "2"),
        ("empty-conditions", "False|False"),
    ] {
        assert_eq!(
            std::fs::read_to_string(directory.path().join(name))?,
            expected,
            "{name}"
        );
    }
    assert!(!directory.path().join("suppressed-handler").exists());
    Ok(())
}

#[test]
fn task_timeout_reaps_descendants_and_never_retries_an_uncertain_modification() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let file = fixture_file(&format!(
        r#"
- hosts: localhost
  gather_facts: false
  tasks:
    - shell: |
        printf x >> {root}/invocations
        setsid sh -c 'echo $$ > {root}/child; sleep 30' &
        wait
      timeout: 1
      retries: 3
      delay: 0
      ignore_errors: true
    - command: touch {root}/later
"#
    ));
    let inventory = local_inventory();
    let output = controller()
        .args([
            "playbook",
            file.path().to_str().unwrap(),
            "-i",
            inventory.path().to_str().unwrap(),
        ])
        .output()?;
    assert!(!output.status.success());
    assert_eq!(
        std::fs::read_to_string(directory.path().join("invocations"))?,
        "x"
    );
    assert!(!directory.path().join("later").exists());
    let pid = std::fs::read_to_string(directory.path().join("child"))?
        .trim()
        .parse::<i32>()?;
    assert_eq!(
        unsafe { libc::kill(pid, 0) },
        -1,
        "timed-out descendant survived"
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    Ok(())
}

#[test]
fn global_check_with_command_prediction_cannot_be_cancelled_by_task_setting() -> Result<()> {
    let directory = TempDir::new()?;
    let file = fixture_file(&format!(
        r#"
- hosts: localhost
  gather_facts: false
  tasks:
    - command:
        argv: [/usr/bin/touch, would-change]
        chdir: {}
        creates: would-change
      check_mode: false
"#,
        directory.path().display()
    ));
    let inventory = local_inventory();
    let output = controller()
        .args([
            "playbook",
            file.path().to_str().unwrap(),
            "-i",
            inventory.path().to_str().unwrap(),
            "--check",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read_dir(directory.path())?.count(), 0);
    Ok(())
}

#[test]
fn sources_follow_playbook_directory_and_runtime_output_is_not_retemplated() -> Result<()> {
    let project = TempDir::new()?;
    let unrelated = TempDir::new()?;
    let inventory_file = local_inventory();
    std::fs::write(project.path().join("input.bin"), [0, 255, 42])?;
    std::fs::write(
        project.path().join("input.j2"),
        "{{ result.stdout }}|{{ missing | default('0012') }}|{{ number_string }}",
    )?;
    let playbook = project.path().join("site.yml");
    std::fs::write(
        &playbook,
        format!(
            r#"- hosts: localhost
  gather_facts: false
  vars:
    number_string: '0012'
  tasks:
    - shell: printf '\173\173 should_never_render \175\175'
      register: result
    - copy:
        src: input.bin
        dest: {0}/binary
    - template:
        src: input.j2
        dest: {0}/rendered
    - copy:
        content: "{{{{ result.stdout }}}}|{{{{ missing | default('0012') }}}}|{{{{ number_string }}}}"
        dest: {0}/copied
    - template:
        content: '{{{{ result.stdout }}}}'
        dest: {0}/inline
"#,
            project.path().display()
        ),
    )?;
    let output = controller()
        .current_dir(unrelated.path())
        .args([
            "playbook",
            playbook.to_str().unwrap(),
            "-i",
            inventory_file.path().to_str().unwrap(),
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read(project.path().join("binary"))?, [0, 255, 42]);
    let expected = "{{ should_never_render }}|0012|0012";
    assert_eq!(
        std::fs::read_to_string(project.path().join("rendered"))?,
        expected
    );
    assert_eq!(
        std::fs::read_to_string(project.path().join("copied"))?,
        expected
    );
    assert_eq!(
        std::fs::read_to_string(project.path().join("inline"))?,
        "{{ should_never_render }}"
    );
    Ok(())
}

#[test]
fn play_and_task_connection_variables_control_the_actual_transport() -> Result<()> {
    let directory = TempDir::new()?;
    let inventory_file =
        fixture_file("[all]\nnode ansible_host=unreachable.invalid ansible_connection=ssh\n");
    let playbook = fixture_file(&format!(
        r#"- hosts: node
  gather_facts: false
  vars:
    transport: local
    ansible_connection: '{{{{ transport }}}}'
    ansible_host: 127.0.0.1
  tasks:
    - copy:
        content: '{{{{ ansible_host }}}}'
        dest: {0}/play
    - copy:
        content: '{{{{ ansible_host }}}}'
        dest: {0}/task
      vars:
        ansible_ssh_host: localhost
"#,
        directory.path().display()
    ));
    let inventory = inventory::parse(inventory_file.path().to_str().unwrap())?;
    run_playbook(playbook.path().to_str().unwrap(), &inventory)?;
    assert_eq!(
        std::fs::read_to_string(directory.path().join("play"))?,
        "127.0.0.1"
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("task"))?,
        "localhost"
    );
    Ok(())
}

#[test]
fn invalid_or_unresolved_connection_values_fail_before_writing() -> Result<()> {
    let inventory_file = local_inventory();
    let directory = TempDir::new()?;
    let marker = directory.path().join("must-not-exist");
    let inventory = inventory::parse(inventory_file.path().to_str().unwrap())?;
    for setting in [
        "ansible_connection: unsupported",
        "ansible_port: true",
        "ansible_user: 42",
        "ansible_become: []",
        "ansible_host: '{{ missing_address }}'",
        "ansible_become_method: su",
    ] {
        let playbook = fixture_file(&format!(
            "- hosts: localhost\n  gather_facts: false\n  tasks:\n    - copy:\n        content: unsafe\n        dest: {}\n      vars:\n        {setting}\n", marker.display()
        ));
        assert!(
            run_playbook(playbook.path().to_str().unwrap(), &inventory).is_err(),
            "{setting}"
        );
        assert!(!marker.exists(), "{setting}");
    }
    Ok(())
}

#[test]
fn facts_obey_variable_precedence_without_becoming_templates() -> Result<()> {
    let directory = TempDir::new()?;
    let inventory_file =
        fixture_file("[all]\nlocalhost ansible_connection=local ansible_system=inventory\n");
    let playbook = fixture_file(&format!(
        r#"- hosts: localhost
  vars:
    ansible_system: play
  tasks:
    - copy:
        content: '{{{{ ansible_system }}}}'
        dest: {0}/play
    - copy:
        content: '{{{{ ansible_system }}}}'
        dest: {0}/task
      vars:
        ansible_system: task
- hosts: localhost
  gather_facts: false
  tasks:
    - copy:
        content: '{{{{ ansible_system }}}}'
        dest: {0}/facts
"#,
        directory.path().display()
    ));
    let inventory = inventory::parse(inventory_file.path().to_str().unwrap())?;
    run_playbook(playbook.path().to_str().unwrap(), &inventory)?;
    for (file, expected) in [("play", "play"), ("task", "task"), ("facts", "Linux")] {
        assert_eq!(
            std::fs::read_to_string(directory.path().join(file))?,
            expected
        );
    }
    Ok(())
}

#[test]
fn loop_variables_resolve_task_paths_and_connection_settings_per_item() -> Result<()> {
    let directory = TempDir::new()?;
    let inventory_file = fixture_file("[all]\nnode ansible_connection=ssh\n");
    let playbook = fixture_file(&format!(
        r#"- hosts: node
  gather_facts: false
  tasks:
    - copy:
        content: '{{{{ item.name }}}}'
        dest: '{{{{ destination }}}}'
      vars:
        destination: '{0}/{{{{ item.name }}}}'
        ansible_connection: '{{{{ item.transport }}}}'
      loop:
        - {{name: first, transport: local}}
        - {{name: second, transport: local}}
"#,
        directory.path().display()
    ));
    let inventory = inventory::parse(inventory_file.path().to_str().unwrap())?;
    run_playbook(playbook.path().to_str().unwrap(), &inventory)?;
    for name in ["first", "second"] {
        assert_eq!(std::fs::read_to_string(directory.path().join(name))?, name);
    }
    Ok(())
}

#[test]
fn registered_results_survive_plays_but_play_and_task_vars_do_not() -> Result<()> {
    let inventory_file = local_inventory();
    let output_dir = TempDir::new()?;
    let marker = output_dir.path().join("result");
    let playbook_file = fixture_file(&format!(
        r#"- hosts: localhost
  gather_facts: false
  vars:
    play_only: secret
  tasks:
    - command: printf carried
      register: previous
      vars:
        task_only: secret
- hosts: localhost
  gather_facts: false
  tasks:
    - copy:
        content: "{{{{ previous.stdout }}}}"
        dest: {}
      when: play_only is not defined and task_only is not defined
"#,
        marker.display()
    ));
    let inventory = inventory::parse(inventory_file.path().to_str().unwrap())?;
    run_playbook(playbook_file.path().to_str().unwrap(), &inventory)?;
    assert_eq!(std::fs::read_to_string(marker)?, "carried");
    Ok(())
}

#[test]
fn failed_hosts_stay_excluded_in_later_plays_while_healthy_hosts_continue() -> Result<()> {
    let inventory_file =
        fixture_file("[local]\nbad ansible_connection=local\ngood ansible_connection=local\n");
    let output_dir = TempDir::new()?;
    let playbook_file = fixture_file(&format!(
        r#"- hosts: bad
  gather_facts: false
  tasks:
    - command: 'false'
- hosts: all
  gather_facts: false
  tasks:
    - copy:
        content: executed
        dest: {}/{{{{ inventory_hostname }}}}
"#,
        output_dir.path().display()
    ));
    let inventory = inventory::parse(inventory_file.path().to_str().unwrap())?;
    assert!(run_playbook(playbook_file.path().to_str().unwrap(), &inventory).is_err());
    assert!(!output_dir.path().join("bad").exists());
    assert_eq!(std::fs::read(output_dir.path().join("good"))?, b"executed");
    Ok(())
}

#[test]
fn explicit_setup_merges_target_facts_across_plays() -> Result<()> {
    let inventory_file = local_inventory();
    let output_dir = TempDir::new()?;
    let marker = output_dir.path().join("facts");
    let playbook_file = fixture_file(&format!(
        r#"- hosts: localhost
  gather_facts: false
  tasks:
    - setup: {{}}
- hosts: localhost
  gather_facts: false
  tasks:
    - copy:
        content: "{{{{ ansible_system }}}}:{{{{ ansible_facts.architecture }}}}:{{{{ ansible_date_time.epoch }}}}"
        dest: {}
"#,
        marker.display()
    ));
    let inventory = inventory::parse(inventory_file.path().to_str().unwrap())?;
    run_playbook(playbook_file.path().to_str().unwrap(), &inventory)?;
    let contents = std::fs::read_to_string(marker)?;
    let parts: Vec<_> = contents.split(':').collect();
    assert_eq!(parts[0], "Linux");
    assert!(!parts[1].is_empty());
    assert!(parts[2].parse::<u64>()? > 0);
    Ok(())
}

#[test]
fn disabled_facts_do_not_invent_target_time() -> Result<()> {
    let inventory_file = local_inventory();
    let output_dir = TempDir::new()?;
    let marker = output_dir.path().join("facts");
    let playbook_file = fixture_file(&format!(
        r#"- hosts: localhost
  gather_facts: false
  tasks:
    - copy:
        content: "{{{{ ansible_date_time | default('absent') }}}}"
        dest: {}
"#,
        marker.display()
    ));
    let inventory = inventory::parse(inventory_file.path().to_str().unwrap())?;
    run_playbook(playbook_file.path().to_str().unwrap(), &inventory)?;
    assert_eq!(std::fs::read_to_string(marker)?, "absent");
    Ok(())
}

#[test]
fn explicit_setup_rejects_arguments_before_later_modifications() -> Result<()> {
    let inventory_file = local_inventory();
    let output_dir = TempDir::new()?;
    let marker = output_dir.path().join("must-not-exist");
    let playbook_file = fixture_file(&format!(
        "- hosts: localhost\n  gather_facts: false\n  tasks:\n    - setup:\n        unsupported: true\n    - command: touch {}\n",
        marker.display()
    ));
    let inventory = inventory::parse(inventory_file.path().to_str().unwrap())?;
    assert!(run_playbook(playbook_file.path().to_str().unwrap(), &inventory).is_err());
    assert!(!marker.exists());
    Ok(())
}

#[test]
fn unreachable_hosts_are_not_ignored_and_are_not_reconnected_in_later_plays() -> Result<()> {
    // A local listener never speaks SSH. Keeping it open makes the handshake
    // failure deterministic without an external host or a reused free port.
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let inventory_file = fixture_file(&format!(
        "[all]\nlost ansible_host=127.0.0.1 ansible_connection=ssh ansible_port={port} ansible_ssh_timeout=1\n"
    ));
    let playbook_file = fixture_file(
        "- hosts: all\n  gather_facts: false\n  tasks:\n    - command: 'true'\n      ignore_errors: true\n- hosts: all\n  tasks:\n    - debug:\n        msg: must-not-run\n",
    );
    let output = controller()
        .args([
            "playbook",
            playbook_file.path().to_str().unwrap(),
            "-i",
            inventory_file.path().to_str().unwrap(),
        ])
        .output()?;
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("unreachable=1 failed=0"), "{stdout}");
    assert_eq!(stdout.matches("PLAY RECAP").count(), 1);
    assert!(!stdout.contains("must-not-run"));
    listener.set_nonblocking(true)?;
    let _connection = listener.accept()?;
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    Ok(())
}

#[test]
fn playbook_error_and_recap_do_not_leak_no_log_payloads() -> Result<()> {
    let inventory_file = local_inventory();
    let playbook_file = fixture_file(
        "- hosts: all\n  gather_facts: false\n  tasks:\n    - shell: echo private-payload >&2; exit 1\n      no_log: true\n- hosts: all\n  gather_facts: false\n  tasks:\n    - debug:\n        msg: must-not-run\n",
    );
    let output = controller()
        .args([
            "playbook",
            playbook_file.path().to_str().unwrap(),
            "-i",
            inventory_file.path().to_str().unwrap(),
        ])
        .output()?;
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;
    assert!(!stdout.contains("private-payload"));
    assert!(!stderr.contains("private-payload"));
    assert!(!stdout.contains("must-not-run"));
    assert!(stdout.contains("failed=1"));
    Ok(())
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
    run_playbook(playbook_path.to_str().unwrap(), &inventory)?;

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
        worker_dir: Some(support::worker_bundle()),
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
        worker_dir: Some(support::worker_bundle()),
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

    let status = controller()
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

    let status = controller()
        .arg("playbook")
        .arg(playbook_file.path())
        .arg("-i")
        .arg(inventory_file.path())
        .status()?;

    assert!(!status.success());
    Ok(())
}

#[test]
fn playbook_cli_prints_the_complete_parse_error_chain() -> Result<()> {
    let inventory_file = local_inventory();
    let playbook_file = fixture_file(
        "---\n- name: unsupported pre tasks\n  hosts: localhost\n  pre_tasks: []\n  tasks: []\n",
    );

    let output = controller()
        .arg("playbook")
        .arg(playbook_file.path())
        .arg("-i")
        .arg(inventory_file.path())
        .output()?;

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("Failed to parse play 1 in document 1"));
    assert!(
        stderr.contains("Play keyword 'pre_tasks' is recognized but not supported"),
        "stderr omitted the root cause: {stderr}"
    );
    Ok(())
}

#[test]
fn test_long_task_name_does_not_panic() -> Result<()> {
    let inventory_file = local_inventory();
    let playbook_file = fixture_file(
        "---\n- name: long title\n  hosts: localhost\n  tasks:\n    - name: this task name is intentionally longer than seventy two characters and must not panic\n      debug:\n        msg: hello\n",
    );
    let inventory = inventory::parse(inventory_file.path().to_str().unwrap())?;

    run_playbook(playbook_file.path().to_str().unwrap(), &inventory)?;
    Ok(())
}

#[test]
fn test_builtin_fqcn_is_accepted() -> Result<()> {
    let inventory_file = local_inventory();
    let playbook_file = fixture_file(
        "---\n- name: fqcn\n  hosts: localhost\n  tasks:\n    - name: builtin debug\n      ansible.builtin.debug:\n        msg: hello\n",
    );
    let inventory = inventory::parse(inventory_file.path().to_str().unwrap())?;

    run_playbook(playbook_file.path().to_str().unwrap(), &inventory)?;
    Ok(())
}

#[test]
fn gathered_architecture_is_available_to_play_variables() -> Result<()> {
    let inventory_file = local_inventory();
    let playbook_file = fixture_file(
        "---\n- name: facts\n  hosts: localhost\n  gather_facts: true\n  vars:\n    normalized_arch: \"{{ 'aarch64' if ansible_architecture in ['aarch64', 'arm64'] else 'x86_64' }}\"\n  tasks:\n    - name: render gathered architecture\n      debug:\n        msg: \"{{ normalized_arch }}\"\n",
    );
    let inventory = inventory::parse(inventory_file.path().to_str().unwrap())?;

    run_playbook(playbook_file.path().to_str().unwrap(), &inventory)?;
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

    let status = controller()
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
        let error = run_playbook(path.to_str().unwrap(), &inventory).unwrap_err();
        let message = format!("{error:#}");
        assert!(
            message.contains("No hosts matched"),
            "documented example {relative_path} failed before host selection: {message}"
        );
    }
    Ok(())
}

#[test]
fn ad_hoc_setup_and_stat_support_native_argument_forms_and_json_results() -> Result<()> {
    let inventory_file = local_inventory();
    let setup = controller()
        .args(["ad-hoc", "localhost", "-m", "ansible.builtin.setup", "-i"])
        .arg(inventory_file.path())
        .output()?;
    assert!(
        setup.status.success(),
        "{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    let setup_stdout = String::from_utf8(setup.stdout)?;
    assert!(setup_stdout.contains("\"ansible_facts\""));
    assert!(setup_stdout.contains("\"ansible_architecture\""));

    let directory = TempDir::new()?;
    let inspected = directory.path().join("missing file");
    let arguments = serde_json::json!({"path": inspected}).to_string();
    let stat = controller()
        .args(["ad-hoc", "localhost", "-m", "stat", "-a", &arguments, "-i"])
        .arg(inventory_file.path())
        .output()?;
    assert!(
        stat.status.success(),
        "{}",
        String::from_utf8_lossy(&stat.stderr)
    );
    let stat_stdout = String::from_utf8(stat.stdout)?;
    assert!(stat_stdout.contains("\"stat\""));
    assert!(stat_stdout.contains("\"exists\": false"));
    Ok(())
}

#[test]
fn ad_hoc_invalid_json_fails_before_opening_a_connection() -> Result<()> {
    let inventory_file = local_inventory();
    let output = controller()
        .args(["ad-hoc", "localhost", "-m", "stat", "-a", "{broken", "-i"])
        .arg(inventory_file.path())
        .output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)?.contains("Invalid JSON module arguments"));
    Ok(())
}

#[test]
fn interrupted_mutation_is_uncertain_not_ignored_or_replayed() -> Result<()> {
    let directory = TempDir::new()?;
    let marker = directory.path().join("executed");
    let later = directory.path().join("must-not-exist");
    let inventory =
        fixture_file("[all]\nlocalhost ansible_connection=local rustsible_command_timeout=1\n");
    let playbook = fixture_file(&format!(
        r#"- hosts: all
  gather_facts: false
  tasks:
    - shell: printf '{{{{ item }}}}' >> {}; printf private-timeout-payload >&2; sleep 3
      loop: [first, second]
      ignore_errors: true
      no_log: true
- hosts: all
  gather_facts: false
  tasks:
    - command: touch {}
"#,
        marker.display(),
        later.display()
    ));
    let output = controller()
        .args([
            "playbook",
            playbook.path().to_str().unwrap(),
            "-i",
            inventory.path().to_str().unwrap(),
            "-vv",
        ])
        .output()?;
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stdout.contains("uncertain=1"), "{stdout}");
    assert!(!stdout.contains("private-timeout-payload"));
    assert!(!stderr.contains("private-timeout-payload"));
    assert_eq!(std::fs::read(marker)?, b"first");
    assert!(!later.exists());
    Ok(())
}

#[test]
fn both_frontends_reject_unknown_arguments_even_in_check_mode() -> Result<()> {
    let inventory =
        fixture_file("[all]\nnode ansible_connection=ssh ansible_host=must-not-connect.invalid\n");
    let playbook = fixture_file("- hosts: all\n  gather_facts: false\n  tasks:\n    - ansible.builtin.command:\n        cmd: echo x\n        unknown: true\n");
    let adhoc = controller()
        .args([
            "ad-hoc",
            "all",
            "-i",
            inventory.path().to_str().unwrap(),
            "-m",
            "ansible.builtin.command",
            "-a",
            r#"{"cmd":"echo x","unknown":true}"#,
            "--check",
        ])
        .output()?;
    let book = controller()
        .args([
            "playbook",
            playbook.path().to_str().unwrap(),
            "-i",
            inventory.path().to_str().unwrap(),
            "--check",
        ])
        .output()?;
    for output in [adhoc, book] {
        assert!(!output.status.success());
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(text.contains("unknown field `unknown`"), "{text}");
        assert!(!text.contains("Failed to connect"));
    }
    Ok(())
}

#[test]
fn native_copy_inline_content_keeps_nul_and_literal_template_bytes() -> Result<()> {
    let temporary = TempDir::new()?;
    let inventory = local_inventory();
    let path = temporary.path().join("binary");
    let content = "literal {{ do_not_evaluate }}\0data";
    // CLI arguments now use the same initial template evaluation as Ansible.
    // Its rendered string must reach the worker verbatim, including NUL/braces.
    let expression = format!("{{{{ {} }}}}", serde_json::to_string(content)?);
    let args = serde_json::json!({"dest":path,"content":expression,"mode":"0600"}).to_string();
    let output = controller()
        .args([
            "ad-hoc",
            "localhost",
            "-i",
            inventory.path().to_str().unwrap(),
            "-m",
            "copy",
            "-a",
            &args,
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read(path)?, content.as_bytes());
    Ok(())
}
