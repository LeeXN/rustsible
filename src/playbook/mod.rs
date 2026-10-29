pub mod filters;
mod handlers;
mod parser;
mod play;
mod strategy;
mod task;
pub(crate) mod templar;

use crate::inventory::Inventory;
use anyhow::{Context, Result};
use log::{debug, error, info};
use std::collections::HashSet;

pub use handlers::Handler;
pub use play::Play;
pub(crate) use play::RunState;
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
            forks: 5,
            worker_dir: None,
            extra_vars: Vec::new(),
            config_file: None,
            role_paths: Vec::new(),
        }
    }
}

pub fn execute(playbook_file: &str, inventory: &Inventory) -> Result<()> {
    execute_with_options(playbook_file, inventory, &ExecutionOptions::default())
}

pub fn execute_with_options(
    playbook_file: &str,
    inventory: &Inventory,
    options: &ExecutionOptions,
) -> Result<()> {
    info!("Loading playbook from file: {}", playbook_file);

    let extra = crate::vars::extra(&options.extra_vars)?;
    let mut playbook = parser::parse_playbook(
        playbook_file,
        &parser::ParseOptions {
            variables: extra.clone(),
            role_paths: options.role_paths.clone(),
            ..Default::default()
        },
    )?;
    let mut inventory = inventory.clone();
    let playbook_source = std::path::Path::new(playbook_file).canonicalize()?;
    if inventory.source.as_ref().and_then(|path| path.parent()) != playbook_source.parent() {
        inventory
            .load_variable_files(playbook_source.parent().expect("Canonical playbook parent"))?;
    }
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
    state.workers = crate::worker::Runtime::new(options.worker_dir.clone());
    state.extra = std::sync::Arc::new(extra);
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
