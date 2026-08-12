pub mod filters;
mod handlers;
mod parser;
mod play;
mod task;
mod templar;

use crate::inventory::Inventory;
use anyhow::Result;
use log::{debug, error, info};
use std::collections::HashSet;

pub use handlers::Handler;
pub use play::Play;
pub use task::{Task, TaskResult};

/// Options that affect a complete playbook execution.
///
/// Keeping these values together prevents CLI switches from being parsed and
/// then silently dropped before they reach the execution engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionOptions {
    pub limit: Option<String>,
    pub check_mode: bool,
    pub forks: usize,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            limit: None,
            check_mode: false,
            forks: 5,
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

    let playbook = parser::parse_playbook(playbook_file)?;
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

    for (index, play) in playbook.plays.iter().enumerate() {
        info!(
            "PLAY [{}] ({}/{})",
            play.name,
            index + 1,
            playbook.plays.len()
        );

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
            play.execute_with_options(&hosts, options.check_mode, options.forks.max(1));

        if let Err(e) = play_result {
            error!("Play '{}' failed: {}", play.name, e);
            // Continue with next play unless fail_fast is enabled
            if play.fail_fast {
                return Err(e);
            }
            failures.push(format!("Play '{}' failed: {}", play.name, e));
        }
    }

    if !failures.is_empty() {
        return Err(anyhow::anyhow!(failures.join("; ")));
    }

    info!("Playbook execution completed");
    Ok(())
}
