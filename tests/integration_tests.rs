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
fn static_imports_keep_nested_sources_bindings_and_per_task_conditions() -> Result<()> {
    let directory = TempDir::new()?;
    let nested = directory.path().join("nested");
    let elsewhere = directory.path().join("elsewhere");
    std::fs::create_dir(&nested)?;
    std::fs::create_dir(&elsewhere)?;
    std::fs::write(nested.join("payload"), "from nested source")?;
    std::fs::write(
        directory.path().join("site.yml"),
        "- import_playbook: '{{ selected }}'\n  vars:\n    greeting: import\n",
    )?;
    std::fs::write(nested.join("vars.yml"), "ready: true")?;
    std::fs::write(
        nested.join("site.yml"),
        r#"
- hosts: localhost
  gather_facts: false
  vars:
    greeting: childplay
  vars_files: [vars.yml]
  tasks:
    - ansible.builtin.import_tasks: deploy.yml
      vars:
        message: '{{ greeting }}'
    - import_tasks: conditional.yml
      when: ready
"#,
    )?;
    std::fs::write(
        nested.join("deploy.yml"),
        format!(
            r#"
- copy:
    src: payload
    dest: {0}/payload
- copy:
    content: '{{{{ message }}}}|{{{{ playbook_dir }}}}'
    dest: {0}/context
"#,
            directory.path().display()
        ),
    )?;
    std::fs::write(
        nested.join("conditional.yml"),
        format!(
            r#"
- set_fact:
    ready: false
- command: touch {}/must-not-run
"#,
            directory.path().display()
        ),
    )?;
    let inventory = local_inventory();
    let output = controller()
        .current_dir(&elsewhere)
        .arg("playbook")
        .arg(directory.path().join("site.yml"))
        .arg("-i")
        .arg(inventory.path())
        .args(["-e", "selected=nested/site.yml"])
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("payload"))?,
        "from nested source"
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("context"))?,
        format!("import|{}", nested.display())
    );
    assert!(!directory.path().join("must-not-run").exists());
    Ok(())
}

#[test]
fn static_import_cycles_and_invalid_children_fail_before_any_target_modification() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let inventory = local_inventory();
    let book = directory.path().join("site.yml");
    std::fs::write(&book, format!("- hosts: localhost\n  gather_facts: false\n  tasks:\n    - command: touch {root}/changed\n    - import_tasks: bad.yml\n"))?;
    for content in ["- import_tasks: bad.yml", "- definitely_not_a_module: {}"] {
        std::fs::write(directory.path().join("bad.yml"), content)?;
        let output = controller()
            .arg("playbook")
            .arg(&book)
            .arg("-i")
            .arg(inventory.path())
            .output()?;
        assert!(!output.status.success());
        assert!(!directory.path().join("changed").exists());
        assert!(String::from_utf8_lossy(&output.stderr).contains("bad.yml"));
    }
    std::fs::write(&book, "- import_playbook: site.yml")?;
    let output = controller()
        .arg("playbook")
        .arg(&book)
        .arg("-i")
        .arg(inventory.path())
        .output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Import cycle"));
    Ok(())
}

#[test]
fn dynamic_includes_select_before_expansion_and_keep_loop_bindings_and_parameter_scope(
) -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    std::fs::create_dir(directory.path().join("tasks"))?;
    for name in ["one", "two"] {
        std::fs::write(
            directory.path().join(format!("tasks/{name}.yml")),
            format!(
                r#"
- set_fact:
    ready: false
- shell: 'printf "%s|%s;" "{{{{ item }}}}" "{{{{ message }}}}" >> {root}/order'
  vars:
    message: child
"#
            ),
        )?;
    }
    std::fs::write(
        directory.path().join("site.yml"),
        format!(
            r#"
- hosts: node
  gather_facts: false
  vars:
    ready: true
  tasks:
    - set_fact:
        target_address: localhost
        ansible_connection: local
    - include_tasks: missing.yml
      when: false
    - include_tasks: tasks/{{{{ item }}}}.yml
      when: ready
      loop: [one, two, one]
      vars:
        message: parent
    - copy:
        content: '{{{{ message | default("absent") }}}}'
        dest: {root}/scope
"#
        ),
    )?;
    let inventory =
        fixture_file("node ansible_host='{{ target_address }}' ansible_connection=ssh\n");
    let output = controller()
        .arg("playbook")
        .arg(directory.path().join("site.yml"))
        .arg("-i")
        .arg(inventory.path())
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("order"))?,
        "one|parent;two|parent;one|parent;"
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("scope"))?,
        "absent"
    );
    Ok(())
}

#[test]
fn selected_dynamic_include_cycles_fail_and_skip_the_next_modification() -> Result<()> {
    let directory = TempDir::new()?;
    let book = directory.path().join("site.yml");
    std::fs::write(
        directory.path().join("recursive.yml"),
        "- include_tasks: recursive.yml\n",
    )?;
    std::fs::write(&book, format!("- hosts: localhost\n  gather_facts: false\n  tasks:\n    - include_tasks: recursive.yml\n    - command: touch {}/later\n", directory.path().display()))?;
    let inventory = local_inventory();
    let output = controller()
        .arg("playbook")
        .arg(book)
        .arg("-i")
        .arg(inventory.path())
        .output()?;
    assert!(!output.status.success());
    assert!(!directory.path().join("later").exists());
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        text.contains("Import cycle") && text.contains("recursive.yml"),
        "{text}"
    );
    Ok(())
}

#[test]
fn roles_load_dependencies_sources_scopes_handlers_and_skip_duplicate_invocations() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    for role in ["common", "web"] {
        for part in [
            "tasks",
            "defaults",
            "vars",
            "handlers",
            "meta",
            "files",
            "templates",
        ] {
            std::fs::create_dir_all(directory.path().join(format!("roles/{role}/{part}")))?;
        }
    }
    std::fs::write(
        directory.path().join("roles/common/defaults/main.yml"),
        "dependency_value: base\n",
    )?;
    std::fs::write(
        directory.path().join("roles/common/tasks/main.yml"),
        format!("- shell: \"printf '%s' 'dependency;' >> {root}/order\"\n"),
    )?;
    std::fs::write(
        directory.path().join("roles/web/meta/main.yml"),
        "dependencies: [common]\n",
    )?;
    std::fs::write(
        directory.path().join("roles/web/defaults/main.yml"),
        "default_value: default\nconflict: default\n",
    )?;
    std::fs::write(
        directory.path().join("roles/web/vars/main.yml"),
        "conflict: rolevars\nrole_value: role\n",
    )?;
    std::fs::write(
        directory.path().join("roles/web/files/payload"),
        "role payload",
    )?;
    std::fs::write(
        directory.path().join("roles/web/templates/config.j2"),
        "{{ default_value }}|{{ conflict }}|{{ role_value }}|{{ dependency_value }}",
    )?;
    std::fs::write(
        directory.path().join("roles/web/tasks/main.yml"),
        format!(
            r#"
- copy:
    src: payload
    dest: {root}/payload
- template:
    src: config.j2
    dest: {root}/config
  vars:
    conflict: task
  notify: reload
- shell: "printf '%s' 'role;' >> {root}/order"
"#
        ),
    )?;
    std::fs::write(directory.path().join("roles/web/handlers/main.yml"), format!("- name: reload\n  copy:\n    content: '{{{{ conflict }}}}'\n    dest: {root}/handler\n"))?;
    std::fs::write(
        directory.path().join("site.yml"),
        format!(
            r#"
- hosts: localhost
  gather_facts: false
  vars:
    conflict: play
  roles:
    - role: web
      conflict: parameter
    - role: web
      conflict: parameter
  tasks:
    - copy:
        content: '{{{{ conflict }}}}|{{{{ default_value }}}}'
        dest: {root}/public
"#
        ),
    )?;
    let inventory = local_inventory();
    let output = controller()
        .arg("playbook")
        .arg(directory.path().join("site.yml"))
        .arg("-i")
        .arg(inventory.path())
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for (name, expected) in [
        ("payload", "role payload"),
        ("config", "default|parameter|role|base"),
        ("public", "rolevars|default"),
        ("handler", "parameter"),
        ("order", "dependency;role;"),
    ] {
        assert_eq!(
            std::fs::read_to_string(directory.path().join(name))?,
            expected,
            "{name}"
        );
    }
    Ok(())
}

#[test]
fn pre_main_post_phases_flush_listeners_in_definition_order_and_last_names_win() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let book = fixture_file(&format!(
        r#"
- hosts: localhost
  gather_facts: false
  pre_tasks:
    - shell: "printf '%s' 'pre;' >> {root}/order"
      notify: all_reloads
  tasks:
    - shell: "printf '%s' 'main;' >> {root}/order"
      notify: last
  post_tasks:
    - shell: "printf '%s' 'post;' >> {root}/order"
      notify: [all_reloads, last]
  handlers:
    - name: first
      listen: all_reloads
      shell: "printf '%s' 'h1;' >> {root}/order"
    - name: second
      listen: all_reloads
      shell: "printf '%s' 'h2;' >> {root}/order"
    - name: last
      shell: "printf '%s' 'shadowed;' >> {root}/order"
    - name: last
      shell: "printf '%s' 'h4;' >> {root}/order"
"#
    ));
    let inventory = local_inventory();
    let output = controller()
        .arg("playbook")
        .arg(book.path())
        .arg("-i")
        .arg(inventory.path())
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("order"))?,
        "pre;h1;h2;main;h4;post;h1;h2;h4;"
    );
    Ok(())
}

#[test]
fn loop_notifications_only_queue_changed_items_and_unknown_names_stop_later_tasks() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let book = fixture_file(&format!(
        r#"
- hosts: localhost
  gather_facts: false
  tasks:
    - command: /bin/true
      changed_when: item == 'two'
      notify: '{{{{ item }}}}'
      loop: [one, two]
  handlers:
    - name: one
      command: touch {root}/one
    - name: two
      command: touch {root}/two
"#
    ));
    let inventory = local_inventory();
    let output = controller()
        .arg("playbook")
        .arg(book.path())
        .arg("-i")
        .arg(inventory.path())
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!directory.path().join("one").exists());
    assert!(directory.path().join("two").exists());
    let bad = fixture_file(&format!("- hosts: localhost\n  gather_facts: false\n  tasks:\n    - command: /bin/true\n      notify: missing\n    - command: touch {root}/later\n"));
    let output = controller()
        .arg("playbook")
        .arg(bad.path())
        .arg("-i")
        .arg(inventory.path())
        .output()?;
    assert!(!output.status.success());
    assert!(!directory.path().join("later").exists());
    Ok(())
}

#[test]
fn dynamic_roles_keep_private_exports_loop_context_and_parameterized_handlers() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    for role in ["private", "base"] {
        for part in ["tasks", "defaults", "vars", "handlers", "meta"] {
            std::fs::create_dir_all(directory.path().join(format!("roles/{role}/{part}")))?;
        }
    }
    std::fs::write(
        directory.path().join("roles/base/defaults/main.yml"),
        "dependency_value: dependency\n",
    )?;
    std::fs::write(
        directory.path().join("roles/private/meta/main.yml"),
        "dependencies: [base]\n",
    )?;
    std::fs::write(
        directory.path().join("roles/private/vars/main.yml"),
        "private_value: private\n",
    )?;
    std::fs::write(
        directory.path().join("roles/private/tasks/main.yml"),
        format!(
            r#"
- copy:
    content: '{{{{ private_value }}}}|{{{{ dependency_value }}}}|{{{{ label }}}}'
    dest: {root}/{{{{ label }}}}
  notify: 'reload {{{{ label }}}}'
"#
        ),
    )?;
    std::fs::write(
        directory.path().join("roles/private/handlers/main.yml"),
        format!(
            r#"
- name: 'reload {{{{ label }}}}'
  copy:
    content: '{{{{ label }}}}'
    dest: {root}/handler-{{{{ label }}}}
"#
        ),
    )?;
    std::fs::write(
        directory.path().join("site.yml"),
        format!(
            r#"
- hosts: localhost
  gather_facts: false
  tasks:
    - include_role:
        name: nonexistent
      when: false
    - include_role:
        name: private
      loop: [one, two]
      vars:
        label: '{{{{ item }}}}'
    - copy:
        content: '{{{{ private_value | default("absent") }}}}|{{{{ dependency_value | default("absent") }}}}'
        dest: {root}/public
"#
        ),
    )?;
    let inventory = local_inventory();
    let output = controller()
        .arg("playbook")
        .arg(directory.path().join("site.yml"))
        .arg("-i")
        .arg(inventory.path())
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("public"))?,
        "absent|absent"
    );
    for name in ["one", "two"] {
        assert_eq!(
            std::fs::read_to_string(directory.path().join(name))?,
            format!("private|dependency|{name}")
        );
        assert_eq!(
            std::fs::read_to_string(directory.path().join(format!("handler-{name}")))?,
            name
        );
    }
    Ok(())
}

#[test]
fn blocks_rescue_failures_run_always_and_reactivate_recovered_hosts() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let book = fixture_file(&format!(
        r#"
- hosts: localhost
  gather_facts: false
  tasks:
    - block:
        - name: deliberate failure
          command: /bin/false
        - command: touch {root}/skipped
      rescue:
        - copy:
            content: '{{{{ ansible_failed_task.name }}}}|{{{{ ansible_failed_result.rc }}}}'
            dest: {root}/rescued
      always:
        - command: touch {root}/always
    - command: touch {root}/continued
- hosts: localhost
  gather_facts: false
  tasks:
    - command: touch {root}/next-play
"#
    ));
    let inventory = local_inventory();
    let output = controller()
        .arg("playbook")
        .arg(book.path())
        .arg("-i")
        .arg(inventory.path())
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("rescued"))?,
        "deliberate failure|1"
    );
    assert!(!directory.path().join("skipped").exists());
    for name in ["always", "continued", "next-play"] {
        assert!(directory.path().join(name).exists(), "{name}");
    }
    assert!(String::from_utf8_lossy(&output.stdout).contains("rescued=1"));
    Ok(())
}

#[test]
fn failed_rescue_still_runs_always_and_preserves_failure_but_nested_rescue_can_recover(
) -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let book = fixture_file(&format!(
        r#"
- hosts: localhost
  gather_facts: false
  tasks:
    - block:
        - block:
            - command: /bin/false
          rescue:
            - command: /bin/false
          always:
            - command: touch {root}/inner-always
      rescue:
        - command: touch {root}/outer-rescue
      always:
        - command: touch {root}/outer-always
    - command: touch {root}/continued
"#
    ));
    let inventory = local_inventory();
    let output = controller()
        .arg("playbook")
        .arg(book.path())
        .arg("-i")
        .arg(inventory.path())
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for name in ["inner-always", "outer-rescue", "outer-always", "continued"] {
        assert!(directory.path().join(name).exists(), "{name}");
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
        "---\n- name: unsupported module defaults\n  hosts: localhost\n  module_defaults: {}\n  tasks: []\n",
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
        stderr.contains("Play keyword 'module_defaults' is recognized but not supported"),
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
        "reuse/main.yml",
        "rolling/playbook.yml",
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
#[test]
fn force_handlers_and_explicit_flush_keep_notifications_without_replaying_completed_handlers(
) -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let book = fixture_file(&format!(
        r#"
- hosts: localhost
  gather_facts: false
  force_handlers: true
  tasks:
    - command: /bin/true
      notify: mark
    - meta: flush_handlers
    - command: /bin/true
      notify: mark
    - command: /bin/false
  handlers:
    - name: mark
      shell: "printf x >> {root}/handler"
"#
    ));
    let inventory = local_inventory();
    let output = controller()
        .arg("playbook")
        .arg(book.path())
        .arg("-i")
        .arg(inventory.path())
        .output()?;
    assert!(!output.status.success());
    assert_eq!(
        std::fs::read_to_string(directory.path().join("handler"))?,
        "xx"
    );
    Ok(())
}

#[test]
fn meta_end_host_end_play_and_clear_errors_keep_play_lifetimes_separate() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let inventory = fixture_file("one ansible_connection=local ansible_host=localhost\ntwo ansible_connection=local ansible_host=localhost\n");
    let book = fixture_file(&format!(
        r#"
- hosts: all
  gather_facts: false
  tasks:
    - meta: end_host
      when: inventory_hostname == 'one'
    - command: touch {root}/first-{{{{ inventory_hostname }}}}
- hosts: all
  gather_facts: false
  tasks:
    - command: touch {root}/second-{{{{ inventory_hostname }}}}
    - meta: end_play
    - command: touch {root}/never
- hosts: all
  gather_facts: false
  tasks:
    - command: /bin/false
      when: inventory_hostname == 'one'
    - meta: clear_host_errors
- hosts: all
  gather_facts: false
  tasks:
    - command: touch {root}/recovered-{{{{ inventory_hostname }}}}
"#
    ));
    let output = controller()
        .arg("playbook")
        .arg(book.path())
        .arg("-i")
        .arg(inventory.path())
        .output()?;
    assert!(
        !output.status.success(),
        "original failure must remain in recap"
    );
    assert!(!directory.path().join("first-one").exists());
    assert!(!directory.path().join("never").exists());
    for name in [
        "first-two",
        "second-one",
        "second-two",
        "recovered-one",
        "recovered-two",
    ] {
        assert!(
            directory.path().join(name).exists(),
            "{name}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}
#[test]
fn unreachable_block_does_not_rescue_or_run_always_and_can_be_cleared_for_a_later_play(
) -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let inventory = fixture_file(&format!("lost ansible_connection=ssh ansible_host=127.0.0.1 ansible_port={port} ansible_ssh_timeout=1\nhealthy ansible_connection=local ansible_host=localhost\n"));
    let book = fixture_file(&format!(
        r#"
- hosts: lost
  gather_facts: false
  tasks:
    - block:
        - command: /bin/true
          ignore_errors: true
      rescue:
        - set_fact:
            wrong_rescue: true
      always:
        - set_fact:
            wrong_always: true
- hosts: all
  gather_facts: false
  tasks:
    - meta: clear_host_errors
- hosts: lost
  gather_facts: false
  vars:
    ansible_connection: local
  tasks:
    - copy:
        content: '{{{{ wrong_rescue | default("no") }}}}|{{{{ wrong_always | default("no") }}}}'
        dest: {root}/recovered
"#
    ));
    let output = controller()
        .arg("playbook")
        .arg(book.path())
        .arg("-i")
        .arg(inventory.path())
        .output()?;
    assert!(!output.status.success());
    assert_eq!(
        std::fs::read_to_string(directory.path().join("recovered"))?,
        "no|no"
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("unreachable=1"));
    Ok(())
}

#[test]
fn ignore_unreachable_is_explicit_and_block_always_obeys_check_and_no_log() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let inventory = fixture_file(&format!("lost ansible_connection=ssh ansible_host=127.0.0.1 ansible_port={port} ansible_ssh_timeout=1\n"));
    let book = fixture_file(&format!(
        r#"
- hosts: all
  gather_facts: false
  tasks:
    - command: /bin/true
      ignore_unreachable: true
    - set_fact:
        ansible_connection: local
    - command: touch {root}/continued
"#
    ));
    let output = controller()
        .arg("playbook")
        .arg(book.path())
        .arg("-i")
        .arg(inventory.path())
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(directory.path().join("continued").exists());
    let safe = fixture_file(&format!(
        r#"
- hosts: all
  gather_facts: false
  vars:
    ansible_connection: local
  tasks:
    - block:
        - debug:
            msg: BLOCK_PRIVATE_SENTINEL
          failed_when: true
      rescue:
        - copy:
            content: secret
            dest: {root}/rescue
      always:
        - copy:
            content: BLOCK_PRIVATE_SENTINEL
            dest: {root}/always
          check_mode: false
      no_log: true
"#
    ));
    let output = controller()
        .arg("playbook")
        .arg(safe.path())
        .arg("-i")
        .arg(inventory.path())
        .arg("--check")
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!directory.path().join("rescue").exists() && !directory.path().join("always").exists());
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!text.contains("BLOCK_PRIVATE_SENTINEL"), "{text}");
    Ok(())
}
#[test]
fn role_cycles_and_plugin_content_are_rejected_during_static_loading() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    for role in ["one", "two"] {
        std::fs::create_dir_all(directory.path().join(format!("roles/{role}/meta")))?;
    }
    std::fs::write(
        directory.path().join("roles/one/meta/main.yml"),
        "dependencies: [two]",
    )?;
    std::fs::write(
        directory.path().join("roles/two/meta/main.yml"),
        "dependencies: [one]",
    )?;
    let book = directory.path().join("site.yml");
    std::fs::write(&book, format!("- hosts: localhost\n  gather_facts: false\n  tasks:\n    - command: touch {root}/changed\n    - import_role:\n        name: one\n"))?;
    let inventory = local_inventory();
    let output = controller()
        .arg("playbook")
        .arg(&book)
        .arg("-i")
        .arg(inventory.path())
        .output()?;
    assert!(!output.status.success());
    assert!(!directory.path().join("changed").exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Role dependency/include cycle"));
    std::fs::write(
        directory.path().join("roles/one/meta/main.yml"),
        "dependencies: []",
    )?;
    std::fs::create_dir(directory.path().join("roles/one/library"))?;
    std::fs::write(
        directory.path().join("roles/one/library/copy.py"),
        "raise RuntimeError('must never execute')",
    )?;
    let output = controller()
        .arg("playbook")
        .arg(&book)
        .arg("-i")
        .arg(inventory.path())
        .output()?;
    assert!(!output.status.success());
    assert!(!directory.path().join("changed").exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Role plugin content is unsupported"));
    Ok(())
}

#[test]
fn imported_roles_expose_static_vars_before_execution_and_dynamic_public_is_explicit() -> Result<()>
{
    let directory = TempDir::new()?;
    let root = directory.path().display();
    std::fs::create_dir_all(directory.path().join("roles/demo/vars"))?;
    std::fs::write(
        directory.path().join("roles/demo/vars/main.yml"),
        "exposed: role",
    )?;
    let book = directory.path().join("site.yml");
    std::fs::write(
        &book,
        format!(
            r#"
- hosts: localhost
  gather_facts: false
  tasks:
    - copy:
        content: '{{{{ exposed }}}}'
        dest: {root}/before
    - import_role:
        name: demo
- hosts: localhost
  gather_facts: false
  tasks:
    - include_role:
        name: demo
        public: true
    - copy:
        content: '{{{{ exposed }}}}'
        dest: {root}/after
"#
        ),
    )?;
    let inventory = local_inventory();
    let output = controller()
        .arg("playbook")
        .arg(book)
        .arg("-i")
        .arg(inventory.path())
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("before"))?,
        "role"
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("after"))?,
        "role"
    );
    Ok(())
}

#[test]
fn imported_playbook_conditions_gate_facts_and_are_rechecked_on_each_task() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let source = directory.path().join("site.yml");
    std::fs::write(&source, "- import_playbook: unreachable.yml\n  when: false\n- import_playbook: conditional.yml\n  vars: {enabled: true}\n  when: enabled\n")?;
    std::fs::write(directory.path().join("unreachable.yml"), "- hosts: localhost\n  vars: {ansible_connection: ssh, ansible_host: unreachable.invalid}\n  tasks:\n    - command: /bin/false\n")?;
    std::fs::write(directory.path().join("conditional.yml"), format!("- hosts: localhost\n  gather_facts: false\n  tasks:\n    - set_fact: {{enabled: false}}\n    - command: touch {root}/must-not-run\n"))?;
    let inventory = local_inventory();
    let output = controller()
        .arg("playbook")
        .arg(source)
        .arg("-i")
        .arg(inventory.path())
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!directory.path().join("must-not-run").exists());
    assert!(String::from_utf8_lossy(&output.stdout).contains("unreachable=0"));
    Ok(())
}

#[test]
fn handler_chains_share_register_state_and_recursive_flush_runs_each_handler_once() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().display();
    let book = fixture_file(&format!(
        r#"
- hosts: localhost
  gather_facts: false
  tasks:
    - command: /bin/true
      notify: first
    - meta: flush_handlers
    - copy:
        content: '{{{{ handler_result.stdout }}}}'
        dest: {root}/registered
  handlers:
    - name: first
      command: /bin/echo from-handler
      register: handler_result
      notify: second
    - name: second
      shell: printf second >> {root}/order
      notify: recursive
    - name: recursive
      meta: flush_handlers
"#
    ));
    let inventory = local_inventory();
    let output = controller()
        .arg("playbook")
        .arg(book.path())
        .arg("-i")
        .arg(inventory.path())
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("order"))?,
        "second"
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("registered"))?,
        "from-handler"
    );
    Ok(())
}

#[test]
fn reusable_deployment_example_predicts_new_directories_without_creating_business_files(
) -> Result<()> {
    let directory = TempDir::new()?;
    let destination = directory.path().join("output");
    let inventory = local_inventory();
    let output = controller()
        .arg("playbook")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/reuse/main.yml"))
        .arg("-i")
        .arg(inventory.path())
        .arg("-e")
        .arg(format!("demo_output={}", destination.display()))
        .arg("--check")
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!destination.exists());
    Ok(())
}
