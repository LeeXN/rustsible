mod context;
pub mod filters;
mod handlers;
mod parser;
mod play;
mod strategy;
mod tags;
mod task;
pub use tags::TagSelection;
pub(crate) mod templar;

use crate::inventory::Inventory;
use anyhow::{Context, Result};
use log::{debug, error, info};
use std::collections::HashSet;

pub use handlers::Handler;
pub use play::Play;
pub(crate) use play::RunState;
pub use strategy::Strategy;
pub(crate) use task::render_module_arguments;
pub use task::{Task, TaskGroup, TaskResult, TaskScope};

/// Options that affect a complete playbook execution.
///
/// Keeping these values together prevents CLI switches from being parsed and
/// then silently dropped before they reach the execution engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionOptions {
    pub limit: Option<String>,
    pub check_mode: bool,
    pub diff: bool,
    pub tags: TagSelection,
    pub credentials: crate::ssh::credentials::Credentials,
    pub vault: std::sync::Arc<crate::vault::Vault>,
    pub extensions: std::sync::Arc<crate::extensions::Registry>,
    pub forks: usize,
    pub worker_dir: Option<std::path::PathBuf>,
    pub extra_vars: Vec<String>,
    pub config_file: Option<std::path::PathBuf>,
    pub role_paths: Vec<std::path::PathBuf>,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            limit: None,
            check_mode: false,
            diff: false,
            tags: TagSelection::default(),
            credentials: Default::default(),
            vault: crate::vault::Vault::current(),
            extensions: crate::extensions::Registry::current(),
            forks: 5,
            worker_dir: None,
            extra_vars: Vec::new(),
            config_file: None,
            role_paths: Vec::new(),
        }
    }
}

struct LoadedPlaybook {
    playbook: parser::Playbook,
    inventory: Inventory,
    extra: std::collections::HashMap<String, serde_yaml::Value>,
    playbook_source: std::path::PathBuf,
}
fn load(
    playbook_file: &str,
    inventory: &Inventory,
    options: &ExecutionOptions,
) -> Result<LoadedPlaybook> {
    let extra = crate::vars::extra(&options.extra_vars)?;
    let playbook = parser::parse_playbook(
        playbook_file,
        &parser::ParseOptions {
            variables: extra.clone(),
            role_paths: options.role_paths.clone(),
            ..Default::default()
        },
    )?;
    let mut inventory = inventory.clone();
    let playbook_source = parser::source_path(playbook_file)?;
    if inventory.source.as_ref().and_then(|path| path.parent()) != playbook_source.parent() {
        inventory
            .load_variable_files(playbook_source.parent().expect("Canonical playbook parent"))?;
    }
    Ok(LoadedPlaybook {
        playbook,
        inventory,
        extra,
        playbook_source,
    })
}

pub fn execute(playbook_file: &str, inventory: &Inventory) -> Result<()> {
    execute_with_options(playbook_file, inventory, &ExecutionOptions::default())
}

pub fn execute_with_options(
    playbook_file: &str,
    inventory: &Inventory,
    options: &ExecutionOptions,
) -> Result<()> {
    let _vault = options.vault.enter();
    let _extensions = options.extensions.enter();
    options
        .vault
        .protect(execute_in_scope(playbook_file, inventory, options))
}

fn execute_in_scope(
    playbook_file: &str,
    inventory: &Inventory,
    options: &ExecutionOptions,
) -> Result<()> {
    info!("Loading playbook from file: {}", playbook_file);

    let LoadedPlaybook {
        mut playbook,
        inventory,
        extra,
        playbook_source,
    } = load(playbook_file, inventory, options)?;
    info!("Playbook contains {} plays", playbook.plays.len());

    let limited_hosts: Option<HashSet<String>> = options.limit.as_deref().map(|pattern| {
        inventory
            .filter_hosts(pattern)
            .into_iter()
            .map(|host| host.name)
            .collect()
    });

    if let (Some(pattern), Some(hosts)) = (&options.limit, &limited_hosts) {
        if hosts.is_empty() {
            return Err(anyhow::anyhow!(
                "No hosts matched the --limit pattern: {}",
                pattern
            ));
        }
    }

    let mut failures = Vec::new();
    let mut state = play::RunState::default();
    state.workers = std::sync::Arc::new(
        crate::worker::Runtime::new(options.worker_dir.clone())
            .with_credentials(options.credentials.clone())
            .with_vault(options.vault.clone())
            .with_extensions(options.extensions.clone())
            .with_diff(options.diff),
    );
    state.extra = std::sync::Arc::new(extra);
    state.tags = std::sync::Arc::new(options.tags.clone());
    state.set_execution_modes(options.check_mode, options.diff);
    state.prepare(
        &inventory,
        playbook_source.parent().context("Playbook has no parent")?,
        options.config_file.as_deref(),
    )?;

    let play_count = playbook.plays.len();
    for (index, play) in playbook.plays.iter_mut().enumerate() {
        info!("PLAY [{}] ({}/{})", play.name, index + 1, play_count);

        let mut hosts = inventory.filter_hosts(&play.hosts);
        if let Some(limited) = &limited_hosts {
            hosts.retain(|host| limited.contains(&host.name));
        }
        if hosts.is_empty() {
            let message = if options.limit.is_some() {
                format!(
                    "No hosts remained for play '{}' after applying --limit",
                    play.name
                )
            } else {
                format!(
                    "No hosts matched for play '{}' with pattern: {}",
                    play.name, play.hosts
                )
            };
            error!("{}", message);
            failures.push(message);
            continue;
        }

        debug!("Play '{}' matched {} hosts", play.name, hosts.len());
        let play_result =
            play.execute_in_run(&hosts, options.check_mode, options.forks, &mut state);

        if let Err(e) = play_result {
            error!("Play '{}' failed: {}", play.name, e);
            // Continue with next play unless fail_fast is enabled
            if play.fail_fast || state.abort_book {
                state.print_recap();
                return Err(e);
            }
            failures.push(format!("Play '{}' failed: {}", play.name, e));
        }
    }

    state.print_recap();
    if !failures.is_empty() {
        return Err(anyhow::anyhow!(failures.join("; ")));
    }

    info!("Playbook execution completed");
    Ok(())
}

#[derive(Clone, Copy)]
pub enum Inspection {
    Syntax,
    Hosts,
    Tasks,
}

/// Static inspection deliberately never launches a worker or opens
/// a connection. Dynamic includes remain selected placeholders, as their paths
/// and contents can depend on facts and results unavailable before execution.
pub fn inspect(
    file: &str,
    inventory: &Inventory,
    options: &ExecutionOptions,
    mode: Inspection,
) -> Result<serde_json::Value> {
    let _vault = options.vault.enter();
    let _extensions = options.extensions.enter();
    options
        .vault
        .protect(inspect_in_scope(file, inventory, options, mode))
}
fn inspect_in_scope(
    file: &str,
    inventory: &Inventory,
    options: &ExecutionOptions,
    mode: Inspection,
) -> Result<serde_json::Value> {
    let LoadedPlaybook {
        playbook,
        inventory,
        extra,
        playbook_source,
    } = load(file, inventory, options)?;
    let mut state = RunState::default();
    state.extra = std::sync::Arc::new(extra);
    state.tags = std::sync::Arc::new(options.tags.clone());
    state.prepare(
        &inventory,
        playbook_source.parent().context("Playbook has no parent")?,
        options.config_file.as_deref(),
    )?;
    let limit = options.limit.as_ref().map(|pattern| {
        inventory
            .filter_hosts(pattern)
            .into_iter()
            .map(|host| host.name)
            .collect::<HashSet<_>>()
    });
    if limit.as_ref().is_some_and(HashSet::is_empty) {
        anyhow::bail!("No hosts matched the --limit pattern");
    }
    let mut plays = Vec::new();
    for play in &playbook.plays {
        let hosts = inventory
            .filter_hosts(&play.hosts)
            .into_iter()
            .filter(|host| {
                limit
                    .as_ref()
                    .is_none_or(|limit| limit.contains(&host.name))
            })
            .collect::<Vec<_>>();
        let selected = |task: &Task, tags: &[String]| -> Result<bool> {
            if let Some(value) = options.tags.literal(tags) {
                return Ok(value);
            }
            for host in &hosts {
                if play.task_selected(task, host, &state)? {
                    return Ok(true);
                }
            }
            Ok(false)
        };
        let mut tasks = Vec::new();
        for task in play
            .pre_tasks
            .iter()
            .chain(&play.tasks)
            .chain(&play.post_tasks)
            .filter(|_| !matches!(mode, Inspection::Hosts))
        {
            inspect_task(
                task,
                &play.tags,
                &selected,
                &mut tasks,
                matches!(mode, Inspection::Syntax),
            )?;
        }
        if matches!(mode, Inspection::Syntax) {
            for handler in &play.handlers {
                inspect_task(&handler.task, &play.tags, &selected, &mut Vec::new(), true)?;
            }
        }
        let hosts = hosts.iter().map(|host| &host.name).collect::<Vec<_>>();
        plays.push(serde_json::json!({"name":play.name,"hosts":hosts,"tasks":tasks}));
    }
    Ok(match mode {
        Inspection::Syntax => serde_json::json!({"valid":true,"playbook":file}),
        Inspection::Hosts => serde_json::Value::Array(
            plays
                .into_iter()
                .map(|mut play| {
                    play.as_object_mut().unwrap().remove("tasks");
                    play
                })
                .collect(),
        ),
        Inspection::Tasks => serde_json::Value::Array(plays),
    })
}
fn inspect_task(
    task: &Task,
    inherited: &[String],
    selection: &impl Fn(&Task, &[String]) -> Result<bool>,
    output: &mut Vec<serde_json::Value>,
    syntax: bool,
) -> Result<()> {
    if task.scope.boundary.is_some() {
        return Ok(());
    }
    let task = task.effective();
    let mut tags = task.tags.clone();
    tags.extend_from_slice(inherited);
    if let Some(group) = &task.group {
        for tasks in [&group.tasks, &group.rescue, &group.always] {
            for child in task.bind_children(tasks) {
                inspect_task(&child, inherited, selection, output, syntax)?;
            }
        }
        return Ok(());
    }
    let dynamic = matches!(task.module.as_str(), "include_tasks" | "include_role");
    if !dynamic {
        let builtin = crate::extensions::Registry::current().module(&task.module, &[])?;
        // Syntax validates names and argument keys even when values need facts;
        // normal shared validation checks resolved types before real execution.
        for key in task.args.keys().filter_map(serde_yaml::Value::as_str) {
            if !(builtin.parameters().contains(&key)
                || matches!(
                    builtin.builtin(),
                    Some(
                        crate::modules::registry::Builtin::Command
                            | crate::modules::registry::Builtin::Shell
                    )
                ) && key == "_raw_params")
                && builtin != crate::modules::registry::Builtin::SetFact
            {
                anyhow::bail!("Unsupported {} parameter: {key}", builtin.name());
            }
        }
    }
    if syntax || selection(&task, &tags)? {
        output.push(serde_json::json!({"name":task.name,"module":task.module,"tags":tags,"source":task.source_file,"dynamic":dynamic}));
    }
    Ok(())
}
