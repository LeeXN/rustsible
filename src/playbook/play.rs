use anyhow::{bail, Context, Result};
use colored::Colorize;
use log::{debug, error, info};
use serde_yaml::{Mapping, Value};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::inventory::Host;
use crate::playbook::{Handler, Task, TaskResult};

/// Play structure representing a set of tasks to run on hosts
#[derive(Debug, Clone)]
pub struct Play {
    pub name: String,
    pub hosts: String,
    pub tasks: Vec<Task>,
    pub handlers: Vec<Handler>,
    pub vars: Mapping,
    /// Distinguishes an explicit play setting from inventory/default values.
    pub become_override: Option<bool>,
    pub is_become: bool, // renamed from 'become' to avoid Rust keyword
    pub become_user: String,
    /// Distinguishes an explicit play user (including `root`) from the default.
    pub become_user_override: Option<String>,
    /// Stop the containing playbook immediately if this play fails.
    pub fail_fast: bool,
    /// Play-level connection override (`local`, `ssh`, or `smart`).
    pub connection: Option<String>,
    /// Ansible gathers facts by default; only an explicit false disables it.
    pub gather_facts: bool,
    #[allow(dead_code)]
    pub tags: Vec<String>, // Keep this for future use
}

#[derive(Debug, Default)]
struct HostStats {
    ok: usize,
    changed: usize,
    failed: usize,
    skipped: usize,
    ignored: usize,
}

impl Play {
    pub fn execute(&self, hosts: &[Host]) -> Result<()> {
        self.execute_with_options(hosts, false, 5)
    }

    pub fn execute_with_options(
        &self,
        hosts: &[Host],
        check_mode: bool,
        forks: usize,
    ) -> Result<()> {
        if forks == 0 {
            bail!(
                "Play '{}' requires forks to be greater than zero",
                self.name
            );
        }

        let mut unique_host_names = HashSet::with_capacity(hosts.len());
        for host in hosts {
            if !unique_host_names.insert(host.name.as_str()) {
                bail!(
                    "Play '{}' received duplicate inventory host '{}'",
                    self.name,
                    host.name
                );
            }
        }

        let start_time = Instant::now();
        info!("PLAY [{}] on {} hosts", self.name, hosts.len());
        println!("\n{}", format!("PLAY [{}]", self.name).bold());

        let mut contexts: HashMap<String, HashMap<String, Value>> = hosts
            .iter()
            .map(|host| (host.name.clone(), self.initial_host_vars(host)))
            .collect();
        let mut active_hosts: HashSet<String> =
            hosts.iter().map(|host| host.name.clone()).collect();
        let mut stats: HashMap<String, HostStats> = hosts
            .iter()
            .map(|host| (host.name.clone(), HostStats::default()))
            .collect();
        let mut failures = Vec::new();
        let mut notified_handlers: HashSet<(String, String)> = HashSet::new();

        if self.gather_facts {
            println!("\nTASK [Gathering Facts] {}", "*".repeat(56).dimmed());
            let gather_task = Task {
                name: "Gathering Facts".to_string(),
                module: "setup".to_string(),
                args: Mapping::new(),
                become_override: None,
                is_become: false,
                become_user: "root".to_string(),
                become_user_override: None,
                connection: None,
                check_mode: None,
                register: None,
                when: None,
                notify: Vec::new(),
                ignore_errors: false,
                no_log: false,
                vars: Mapping::new(),
                tags: Vec::new(),
                loop_items: None,
                loop_var_name: None,
                index_var_name: None,
            };
            let gathering_results = Self::execute_task_on_hosts(
                self,
                &gather_task,
                hosts.iter().collect(),
                &contexts,
                check_mode,
                forks,
            );
            for (host, _, gathering_result) in gathering_results {
                let mut result = match gathering_result {
                    Ok(result) => result,
                    Err(error) => {
                        let mut result = TaskResult::new(&host.name);
                        result.failed = true;
                        result.msg = format!("Fact gathering failed: {error}");
                        crate::playbook::task::print_task_result(
                            &host.name,
                            "Gathering Facts",
                            &result,
                            "0.00s",
                        );
                        result
                    }
                };

                if !result.failed {
                    let merge_result = contexts
                        .get_mut(&host.name)
                        .context("Internal play state lost a fact context")
                        .and_then(|context| Self::merge_facts(context, &result));
                    if let Err(error) = merge_result {
                        result.failed = true;
                        result.msg = error.to_string();
                    }
                }

                let host_stats = stats
                    .get_mut(&host.name)
                    .context("Internal play state lost fact-gathering statistics")?;
                if result.failed {
                    host_stats.failed += 1;
                    active_hosts.remove(&host.name);
                    failures.push(format!(
                        "Task 'Gathering Facts' failed on host '{}': {}",
                        host.name, result.msg
                    ));
                } else {
                    host_stats.ok += 1;
                }
            }
        }

        // Execute all tasks in order
        for (task_index, task) in self.tasks.iter().enumerate() {
            if active_hosts.is_empty() {
                debug!(
                    "Stopping play '{}' before task {} because no active hosts remain",
                    self.name,
                    task_index + 1
                );
                break;
            }
            debug!(
                "Executing task {} of {}: {}",
                task_index + 1,
                self.tasks.len(),
                task.name
            );

            // 更接近ansible风格的任务标题
            let title_width = task.name.chars().count() + 8;
            println!(
                "\nTASK [{}] {}",
                task.name,
                "*".repeat(80usize.saturating_sub(title_width)).dimmed()
            );

            let runnable_hosts: Vec<&Host> = hosts
                .iter()
                .filter(|host| active_hosts.contains(&host.name))
                .collect();
            let task_results = Self::execute_task_on_hosts(
                self,
                task,
                runnable_hosts,
                &contexts,
                check_mode,
                forks,
            );

            for (host, effective_task, task_result) in task_results {
                let result = match task_result {
                    Ok(result) => result,
                    Err(error) => {
                        if effective_task.no_log {
                            error!("A no_log task failed on host {}", host.name);
                        } else {
                            error!("Task execution failed on host {}: {}", host.name, error);
                        }
                        let mut result = TaskResult::new(&host.name);
                        result.failed = true;
                        result.no_log = effective_task.no_log;
                        result.msg = if effective_task.no_log {
                            "output censored because no_log is enabled".to_string()
                        } else {
                            format!("Task execution error: {}", error)
                        };
                        crate::playbook::task::print_task_result(
                            &host.name,
                            &effective_task.name,
                            &result,
                            "0.00s",
                        );
                        result
                    }
                };

                if let Some(register_var) = &effective_task.register {
                    let host_context = contexts
                        .get_mut(&host.name)
                        .with_context(|| {
                            format!(
                                "Internal play state error: variable context for host '{}' is missing while registering task '{}'",
                                host.name, effective_task.name
                            )
                        })?;
                    host_context.insert(register_var.clone(), Self::registered_value(&result));
                }

                if result.changed && !result.failed {
                    for handler_name in &effective_task.notify {
                        debug!(
                            "Handler '{}' notified on host '{}' by task '{}'",
                            handler_name, host.name, effective_task.name
                        );
                        notified_handlers.insert((host.name.clone(), handler_name.clone()));
                    }
                }

                let host_stats = stats
                    .get_mut(&host.name)
                    .with_context(|| {
                        format!(
                            "Internal play state error: statistics for host '{}' are missing after task '{}'",
                            host.name, effective_task.name
                        )
                    })?;
                if result.failed {
                    if effective_task.ignore_errors {
                        host_stats.ignored += 1;
                    } else {
                        host_stats.failed += 1;
                        active_hosts.remove(&host.name);
                        let failure_message = if effective_task.no_log {
                            "output censored because no_log is enabled"
                        } else {
                            result.msg.as_str()
                        };
                        failures.push(format!(
                            "Task '{}' failed on host '{}': {}",
                            effective_task.name, host.name, failure_message
                        ));
                    }
                } else if result.skipped {
                    host_stats.skipped += 1;
                } else {
                    host_stats.ok += 1;
                    if result.changed {
                        host_stats.changed += 1;
                    }
                }
            }
        }

        if !notified_handlers.is_empty() {
            info!("Running notified handlers");
            println!("\n{}", "RUNNING HANDLERS".bold());
            println!(
                "{}\n",
                format!("{} host/handler notifications", notified_handlers.len()).dimmed()
            );

            let known_handlers: HashSet<&str> = self
                .handlers
                .iter()
                .map(|handler| handler.task.name.as_str())
                .collect();
            for (host_name, handler_name) in &notified_handlers {
                if !known_handlers.contains(handler_name.as_str()) {
                    failures.push(format!(
                        "Handler '{}' notified on host '{}' but was not defined",
                        handler_name, host_name
                    ));
                }
            }

            for handler in &self.handlers {
                let handler_name = &handler.task.name;
                let handler_hosts: Vec<&Host> = hosts
                    .iter()
                    .filter(|host| {
                        active_hosts.contains(&host.name)
                            && notified_handlers
                                .contains(&(host.name.clone(), handler_name.clone()))
                    })
                    .collect();
                let handler_results = Self::execute_task_on_hosts(
                    self,
                    &handler.task,
                    handler_hosts,
                    &contexts,
                    check_mode,
                    forks,
                );

                for (host, effective_handler, handler_result) in handler_results {
                    debug!("Running handler '{}' on host '{}'", handler_name, host.name);

                    let failed_message = match handler_result {
                        Ok(result) if result.failed => Some(result.msg),
                        Ok(result) => {
                            let host_stats = stats
                                .get_mut(&host.name)
                                .with_context(|| {
                                    format!(
                                        "Internal play state error: statistics for host '{}' are missing after handler '{}'",
                                        host.name, handler_name
                                    )
                                })?;
                            if result.skipped {
                                host_stats.skipped += 1;
                            } else {
                                host_stats.ok += 1;
                                if result.changed {
                                    host_stats.changed += 1;
                                }
                            }
                            None
                        }
                        Err(error) => Some(error.to_string()),
                    };

                    if let Some(message) = failed_message {
                        if effective_handler.no_log {
                            error!(
                                "A no_log handler '{}' failed on host {}",
                                handler_name, host.name
                            );
                        } else {
                            error!(
                                "Handler '{}' failed on host {}: {}",
                                handler_name, host.name, message
                            );
                        }
                        let host_stats = stats
                            .get_mut(&host.name)
                            .with_context(|| {
                                format!(
                                    "Internal play state error: statistics for host '{}' are missing after handler '{}' failed",
                                    host.name, handler_name
                                )
                            })?;
                        host_stats.failed += 1;
                        active_hosts.remove(&host.name);
                        let failure_message = if effective_handler.no_log {
                            "output censored because no_log is enabled"
                        } else {
                            message.as_str()
                        };
                        failures.push(format!(
                            "Handler '{}' failed on host '{}': {}",
                            handler_name, host.name, failure_message
                        ));
                    }
                }
            }
        }

        let elapsed = start_time.elapsed();
        let execution_time = format!("{:.2}s", elapsed.as_secs_f64());

        info!("Play '{}' completed in {}", self.name, execution_time);

        println!("\n{}", "PLAY RECAP".bold());
        println!("{}", "*".repeat(80).dimmed());

        let average_seconds = if hosts.is_empty() {
            0.0
        } else {
            elapsed.as_secs_f64() / hosts.len() as f64
        };
        for host in hosts {
            let host_stats = stats.get(&host.name).with_context(|| {
                format!(
                    "Internal play state error: statistics for host '{}' are missing during recap",
                    host.name
                )
            })?;
            let host_attrs = [
                format!("{}={}", "ok".green().bold(), host_stats.ok),
                format!("{}={}", "changed".yellow().bold(), host_stats.changed),
                format!("{}={}", "failed".red().bold(), host_stats.failed),
                format!("{}={}", "skipped".yellow().bold(), host_stats.skipped),
                format!("ignored={}", host_stats.ignored),
                format!("{}={:.2}s", "time".cyan(), average_seconds),
            ];

            println!("{:<30} : {}", host.name.bold(), host_attrs.join("  "));
        }

        println!("\nPlay execution completed in {}", execution_time);

        if failures.is_empty() {
            Ok(())
        } else {
            Err(anyhow::anyhow!(failures.join("; ")))
        }
    }

    /// Execute one task concurrently across hosts, while keeping the task
    /// sequence itself ordered. Work is batched to avoid creating an
    /// unbounded number of operating-system threads for a large inventory.
    fn execute_task_on_hosts<'a>(
        play: &Play,
        task: &Task,
        hosts: Vec<&'a Host>,
        contexts: &HashMap<String, HashMap<String, Value>>,
        check_mode: bool,
        forks: usize,
    ) -> Vec<(&'a Host, Task, Result<TaskResult>)> {
        let mut results = Vec::with_capacity(hosts.len());
        for batch in hosts.chunks(forks) {
            let completed = std::thread::scope(|scope| {
                let mut workers = Vec::with_capacity(batch.len());
                for host in batch.iter().copied() {
                    let host_vars = contexts.get(&host.name);
                    let effective_task = play.apply_play_settings(task, host);
                    let worker_task = effective_task.clone();
                    let worker = scope.spawn(move || match host_vars {
                        Some(vars) => worker_task.execute_with_options(host, vars, check_mode),
                        None => Err(anyhow::anyhow!(
                            "Internal error: no variable context for host '{}'",
                            host.name
                        )),
                    });
                    workers.push((host, effective_task, worker));
                }

                workers
                    .into_iter()
                    .map(|(host, effective_task, worker)| {
                        let result = match worker.join() {
                            Ok(result) => result,
                            Err(_) => Err(anyhow::anyhow!(
                                "Task worker panicked while processing host '{}'",
                                host.name
                            )),
                        };
                        (host, effective_task, result)
                    })
                    .collect::<Vec<_>>()
            });
            results.extend(completed);
        }
        results
    }

    fn apply_play_settings(&self, task: &Task, host: &Host) -> Task {
        let mut effective_task = task.clone();
        effective_task.is_become = task
            .become_override
            .or(self.become_override)
            .or_else(|| host.get_become())
            .unwrap_or(false);
        effective_task.become_user = task
            .become_user_override
            .clone()
            .or_else(|| self.become_user_override.clone())
            .or_else(|| host.get_become_user().cloned())
            .unwrap_or_else(|| "root".to_string());
        if effective_task.connection.is_none() {
            effective_task.connection = self.connection.clone();
        }
        effective_task
    }

    fn initial_host_vars(&self, host: &Host) -> HashMap<String, Value> {
        let mut vars = HashMap::new();
        for (key, value) in &host.inherited_variables {
            vars.insert(key.clone(), Value::String(value.clone()));
        }
        for (key, value) in &host.typed_inherited_variables {
            vars.insert(key.clone(), value.clone());
        }
        for (key, value) in &host.variables {
            vars.insert(key.clone(), Value::String(value.clone()));
        }
        for (key, value) in &host.typed_variables {
            vars.insert(key.clone(), value.clone());
        }
        // Play vars have higher precedence than inventory group/host vars;
        // task vars are merged one level later by Task::merge_task_vars.
        for (key, value) in &self.vars {
            if let Value::String(key) = key {
                vars.insert(key.clone(), value.clone());
            }
        }
        vars.insert(
            "ansible_hostname".to_string(),
            Value::String(host.hostname.clone()),
        );
        vars.insert(
            "inventory_hostname".to_string(),
            Value::String(host.name.clone()),
        );
        vars.insert(
            "ansible_host".to_string(),
            Value::String(host.hostname.clone()),
        );
        vars.insert("ansible_port".to_string(), Value::Number(host.port.into()));
        vars
    }

    fn merge_facts(context: &mut HashMap<String, Value>, result: &TaskResult) -> Result<()> {
        let facts = result
            .values
            .get("ansible_facts")
            .and_then(Value::as_mapping)
            .context("Fact gathering did not return ansible_facts")?;
        context.insert("ansible_facts".to_string(), Value::Mapping(facts.clone()));
        for (key, value) in facts {
            let Value::String(key) = key else {
                bail!("Fact names must be strings");
            };
            context.insert(format!("ansible_{key}"), value.clone());
        }
        Ok(())
    }

    fn registered_value(result: &TaskResult) -> Value {
        let mut value = Mapping::new();
        value.insert(
            Value::String("changed".to_string()),
            Value::Bool(result.changed),
        );
        value.insert(
            Value::String("failed".to_string()),
            Value::Bool(result.failed),
        );
        value.insert(
            Value::String("skipped".to_string()),
            Value::Bool(result.skipped),
        );
        value.insert(
            Value::String("msg".to_string()),
            Value::String(result.msg.clone()),
        );
        for (key, item) in &result.values {
            value.insert(Value::String(key.clone()), item.clone());
        }
        Value::Mapping(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::create_test_host;
    use serde_yaml::Mapping;
    use std::fs;
    use tempfile::tempdir;

    fn create_local_host() -> Host {
        create_test_host("localhost", "localhost", 22, None, None)
    }

    fn create_test_play() -> Play {
        Play {
            name: "Test Play".to_string(),
            hosts: "localhost".to_string(),
            tasks: Vec::new(),
            handlers: Vec::new(),
            vars: Mapping::new(),
            become_override: None,
            is_become: false,
            become_user: "root".to_string(),
            become_user_override: None,
            fail_fast: false,
            connection: None,
            gather_facts: false,
            tags: Vec::new(),
        }
    }

    fn create_command_task(name: &str, command: &str) -> Task {
        let mut args = Mapping::new();
        args.insert(
            Value::String("_raw_params".to_string()),
            Value::String(command.to_string()),
        );

        Task {
            name: name.to_string(),
            module: "command".to_string(),
            args,
            become_override: None,
            is_become: false,
            become_user: "root".to_string(),
            become_user_override: None,
            connection: None,
            check_mode: None,
            register: None,
            when: None,
            notify: Vec::new(),
            ignore_errors: false,
            no_log: false,
            vars: Mapping::new(),
            tags: Vec::new(),
            loop_items: None,
            loop_var_name: None,
            index_var_name: None,
        }
    }

    #[test]
    fn play_vars_override_inventory_and_preserve_typed_values() {
        let mut host = create_local_host();
        host.set_variable("priority", "inventory");
        host.set_typed_variable(
            "items",
            Value::Sequence(vec![Value::String("one".to_string())]),
        );
        let mut play = create_test_play();
        play.vars.insert(
            Value::String("priority".to_string()),
            Value::String("play".to_string()),
        );

        let vars = play.initial_host_vars(&host);
        assert_eq!(vars.get("priority"), Some(&Value::String("play".into())));
        assert!(matches!(vars.get("items"), Some(Value::Sequence(_))));
    }

    fn create_shell_task(name: &str, command: &str) -> Task {
        let mut task = create_command_task(name, command);
        task.module = "shell".to_string();
        task
    }

    #[test]
    fn rejects_zero_forks_with_a_diagnostic_error() {
        let play = create_test_play();
        let error = play.execute_with_options(&[], false, 0).unwrap_err();
        assert!(error.to_string().contains("forks"));
        assert!(error.to_string().contains("greater than zero"));
    }

    #[test]
    fn rejects_duplicate_host_names_before_building_play_state() {
        let play = create_test_play();
        let hosts = vec![create_local_host(), create_local_host()];
        let error = play.execute(&hosts).unwrap_err();
        assert!(error.to_string().contains("duplicate inventory host"));
        assert!(error.to_string().contains("localhost"));
    }

    #[test]
    fn test_play_execute_sequential() {
        let mut play = create_test_play();
        play.tasks.push(create_command_task("Task 1", "echo task1"));
        play.tasks.push(create_command_task("Task 2", "echo task2"));

        let hosts = vec![create_local_host()];
        let result = play.execute(&hosts);

        assert!(result.is_ok());
    }

    #[test]
    fn test_play_execute_failure() {
        let mut play = create_test_play();
        // False command fails with exit code 1
        play.tasks.push(create_command_task("Fail Task", "false"));

        let hosts = vec![create_local_host()];
        let result = play.execute(&hosts);

        // Should return error because task failed
        assert!(result.is_err());
    }

    #[test]
    fn test_play_execute_ignore_errors() {
        let mut play = create_test_play();
        let mut task = create_command_task("Fail Task", "false");
        task.ignore_errors = true;
        play.tasks.push(task);
        play.tasks
            .push(create_command_task("Success Task", "echo success"));

        let hosts = vec![create_local_host()];
        let result = play.execute(&hosts);

        // Should succeed because ignore_errors is true
        assert!(result.is_ok());
    }

    #[test]
    fn test_play_execute_when_condition() {
        let mut play = create_test_play();
        let mut task = create_command_task("Conditional Task", "echo ran");
        // Condition that evaluates to false
        task.when = Some(Value::String("1 == 2".to_string()));
        play.tasks.push(task);

        let hosts = vec![create_local_host()];
        let result = play.execute(&hosts);

        assert!(result.is_ok());
        // In a real test we'd check if it ran, but here we just check it didn't crash
        // and returned success (skipped tasks are successful)
    }

    #[test]
    fn task_can_explicitly_disable_play_become() {
        let mut play = create_test_play();
        play.become_override = Some(true);
        play.is_become = true;
        play.become_user_override = Some("admin".to_string());
        play.become_user = "admin".to_string();

        let inherited = create_command_task("Inherited", "true");
        let host = create_local_host();
        let inherited = play.apply_play_settings(&inherited, &host);
        assert!(inherited.is_become);
        assert_eq!(inherited.become_user, "admin");

        let mut disabled = create_command_task("Disabled", "true");
        disabled.become_override = Some(false);
        let disabled = play.apply_play_settings(&disabled, &host);
        assert!(!disabled.is_become);
    }

    #[test]
    fn become_settings_follow_task_play_inventory_precedence() {
        let mut host = create_local_host();
        host.set_variable("ansible_become", "true");
        host.set_variable("ansible_become_user", "inventory-user");
        let mut play = create_test_play();
        let task = create_command_task("Inherited", "true");

        let inherited = play.apply_play_settings(&task, &host);
        assert!(inherited.is_become);
        assert_eq!(inherited.become_user, "inventory-user");

        play.become_override = Some(false);
        play.become_user_override = Some("root".to_string());
        let play_override = play.apply_play_settings(&task, &host);
        assert!(!play_override.is_become);
        assert_eq!(play_override.become_user, "root");

        let mut task_override = task;
        task_override.become_override = Some(true);
        task_override.become_user_override = Some("task-user".to_string());
        let task_override = play.apply_play_settings(&task_override, &host);
        assert!(task_override.is_become);
        assert_eq!(task_override.become_user, "task-user");
    }

    #[test]
    fn test_registered_result_persists_between_tasks() {
        let mut play = create_test_play();
        let mut capture = create_command_task("Capture", "printf registered");
        capture.register = Some("captured".to_string());
        play.tasks.push(capture);
        play.tasks.push(create_command_task(
            "Use registered value",
            "test \"{{ captured.stdout }}\" = registered",
        ));

        assert!(play.execute(&[create_local_host()]).is_ok());
    }

    #[test]
    fn test_handler_notification_is_scoped_to_triggering_host() {
        let directory = tempdir().unwrap();
        let output = directory.path().join("handler-hosts");
        let mut play = create_test_play();
        let mut trigger = create_command_task("Trigger", "true");
        trigger.when = Some(Value::String("inventory_hostname == 'first'".to_string()));
        trigger.notify.push("record handler".to_string());
        play.tasks.push(trigger);
        play.handlers.push(Handler {
            task: create_shell_task(
                "record handler",
                &format!(
                    "printf '%s' '{{{{ inventory_hostname }}}}' >> {}",
                    output.display()
                ),
            ),
        });
        let hosts = vec![
            create_test_host("first", "localhost", 22, None, None),
            create_test_host("second", "localhost", 22, None, None),
        ];

        assert!(play.execute(&hosts).is_ok());
        assert_eq!(fs::read_to_string(output).unwrap(), "first");
    }

    #[test]
    fn test_failed_host_is_removed_but_healthy_hosts_continue() {
        let directory = tempdir().unwrap();
        let output = directory.path().join("continued-hosts");
        let mut play = create_test_play();
        let mut fail_first = create_command_task("Fail first", "false");
        fail_first.when = Some(Value::String("inventory_hostname == 'first'".to_string()));
        play.tasks.push(fail_first);
        play.tasks.push(create_shell_task(
            "Record survivors",
            &format!(
                "printf '%s' '{{{{ inventory_hostname }}}}' >> {}",
                output.display()
            ),
        ));
        let hosts = vec![
            create_test_host("first", "localhost", 22, None, None),
            create_test_host("second", "localhost", 22, None, None),
        ];

        assert!(play.execute(&hosts).is_err());
        assert_eq!(fs::read_to_string(output).unwrap(), "second");
    }

    #[test]
    fn play_stops_scheduling_when_every_host_has_failed() {
        let directory = tempdir().unwrap();
        let marker = directory.path().join("must-not-run");
        let mut play = create_test_play();
        play.tasks.push(create_command_task("Fail", "false"));
        play.tasks.push(create_command_task(
            "No active hosts",
            &format!("touch {}", marker.display()),
        ));

        assert!(play.execute(&[create_local_host()]).is_err());
        assert!(!marker.exists());
    }
}
