use rustsible::{cli, inventory, modules, playbook};

use anyhow::{Context, Result};
use env_logger::Builder;
use log::{info, LevelFilter};
use std::io::Write;

fn main() -> Result<()> {
    if let Some(result) = rustsible_native::rpc::handle_runner_mode() {
        return result;
    }
    let matches = cli::build_cli().get_matches();
    let vault = rustsible::vault::Vault::from_cli(&matches)?;
    let _scope = vault.enter();
    if let Some(("vault", sub)) = matches.subcommand() {
        return vault.run_cli(sub);
    }
    if let Some(("collection", sub)) = matches.subcommand() {
        return rustsible::extensions::run_cli(sub);
    }
    vault.protect(run(matches))
}
fn run(matches: clap::ArgMatches) -> Result<()> {
    let mut config = rustsible::config::Config::load(
        matches
            .get_one::<std::path::PathBuf>("config")
            .map(std::path::PathBuf::as_path),
    )?;

    if let Some((_, sub)) = matches.subcommand() {
        if let Ok(Some(path)) = sub.try_get_one::<std::path::PathBuf>("private-key") {
            config.set_private_key(path)?;
        }
    }

    // Set log level based on the number of verbose flags
    let log_level = match matches.subcommand() {
        Some((_, sub_matches)) => match if sub_matches.get_count("verbose") > 0 {
            sub_matches.get_count("verbose")
        } else {
            config.verbosity.unwrap_or(0)
        } {
            0 => LevelFilter::Off,
            1 => LevelFilter::Info,
            2 => LevelFilter::Debug,
            3 => LevelFilter::Trace,
            _ => LevelFilter::Trace,
        },
        _ => LevelFilter::Info,
    };

    // Custom log format
    let mut builder = Builder::new();
    builder
        .format(|buf, record| {
            if rustsible::vault::Vault::current().used() {
                return writeln!(
                    buf,
                    "[{}] Vault-protected diagnostic censored",
                    record.level()
                );
            }
            writeln!(
                buf,
                "{} [{}] {} - {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                record.level(),
                record.target(),
                record.args()
            )
        })
        .filter_level(log_level)
        .init();

    let sub = matches.subcommand().context("A command is required")?.1;
    let paths = sub
        .get_many::<std::path::PathBuf>("collections-path")
        .map(|paths| paths.cloned().collect::<Vec<_>>())
        .unwrap_or_else(|| config.collection_paths.clone());
    let project = sub
        .try_get_one::<String>("playbook")
        .ok()
        .flatten()
        .filter(|file| std::path::Path::new(file).is_file())
        .and_then(|file| std::path::Path::new(file).parent())
        .map(std::path::Path::to_path_buf)
        .unwrap_or(std::env::current_dir()?);
    let extensions = rustsible::extensions::Registry::new(&paths, &project)?;
    let _extensions = extensions.enter();
    info!("Starting Rustsible");

    match matches.subcommand() {
        Some(("playbook", sub_matches)) => {
            let playbook_file =
                required_cli_string(sub_matches.get_one::<String>("playbook"), "playbook file")?;

            info!("Running playbook: {}", playbook_file);
            let inventory = load_inventory(sub_matches, &config)?;
            let forks = config.host_limit(sub_matches)?;
            let options = playbook::ExecutionOptions {
                role_paths: config.role_paths.clone(),
                limit: sub_matches.get_one::<String>("limit").cloned(),
                config_file: config.file.clone(),
                extra_vars: sub_matches
                    .get_many::<String>("extra-vars")
                    .map(|values| values.cloned().collect())
                    .unwrap_or_default(),
                check_mode: sub_matches.get_flag("check"),
                diff: sub_matches.get_flag("diff"),
                vault: rustsible::vault::Vault::current(),
                extensions: extensions.clone(),
                credentials: if sub_matches.get_flag("syntax-check")
                    || sub_matches.get_flag("list-hosts")
                    || sub_matches.get_flag("list-tasks")
                {
                    Default::default()
                } else {
                    credentials(sub_matches)?
                },
                tags: playbook::TagSelection::new(
                    &sub_matches
                        .get_many::<String>("tags")
                        .map(|values| values.cloned().collect::<Vec<_>>())
                        .unwrap_or_default(),
                    &sub_matches
                        .get_many::<String>("skip-tags")
                        .map(|values| values.cloned().collect::<Vec<_>>())
                        .unwrap_or_default(),
                ),
                forks,
                worker_dir: sub_matches
                    .get_one::<std::path::PathBuf>("worker-dir")
                    .cloned()
                    .or_else(|| config.worker_dir.clone()),
            };
            let inspection = if sub_matches.get_flag("syntax-check") {
                Some(playbook::Inspection::Syntax)
            } else if sub_matches.get_flag("list-hosts") {
                Some(playbook::Inspection::Hosts)
            } else if sub_matches.get_flag("list-tasks") {
                Some(playbook::Inspection::Tasks)
            } else {
                None
            };
            let result = if let Some(mode) = inspection {
                playbook::inspect(playbook_file, &inventory, &options, mode).and_then(|report| {
                    println!("{}", serde_json::to_string_pretty(&report)?);
                    Ok(())
                })
            } else {
                playbook::execute_with_options(playbook_file, &inventory, &options)
            };

            if let Err(e) = result {
                eprintln!("Error executing playbook: {e:#}");
                std::process::exit(1);
            }
        }
        Some(("ad-hoc", sub_matches)) => {
            let module =
                required_cli_string(sub_matches.get_one::<String>("module"), "module name")?;
            let args =
                required_cli_string(sub_matches.get_one::<String>("args"), "module arguments")?;
            let host_pattern =
                required_cli_string(sub_matches.get_one::<String>("pattern"), "host pattern")?;

            info!("Running ad-hoc command with module: {}", module);
            let inventory = load_inventory(sub_matches, &config)?;
            let hosts = inventory.filter_hosts(host_pattern);

            if hosts.is_empty() {
                eprintln!("No hosts matched the pattern: {}", host_pattern);
                std::process::exit(1);
            }

            let forks = config.host_limit(sub_matches)?;
            let options = modules::AdHocOptions {
                vault: rustsible::vault::Vault::current(),
                extensions: extensions.clone(),
                credentials: credentials(sub_matches)?,
                inventory: Some(std::sync::Arc::new(inventory.clone())),
                config_file: config.file.clone(),
                // Omitting --become deliberately leaves the decision to each
                // host's inventory. --become-user selects the target account
                // but, like Ansible, does not itself enable escalation.
                become_override: sub_matches.get_flag("become").then_some(true),
                extra_vars: sub_matches
                    .get_many::<String>("extra-vars")
                    .map(|values| values.cloned().collect())
                    .unwrap_or_default(),
                become_user: sub_matches.get_one::<String>("become-user").cloned(),
                background: *sub_matches.get_one::<u64>("background").unwrap_or(&0),
                poll: *sub_matches.get_one::<u64>("poll").unwrap_or(&15),
                check_mode: sub_matches.get_flag("check"),
                diff: sub_matches.get_flag("diff"),
                forks,
                worker_dir: sub_matches
                    .get_one::<std::path::PathBuf>("worker-dir")
                    .cloned()
                    .or_else(|| config.worker_dir.clone()),
            };
            let result = modules::run_adhoc_with_options(&hosts, module, args, &options);
            if let Err(e) = result {
                eprintln!("Error executing ad-hoc command: {e:#}");
                std::process::exit(1);
            }
        }
        Some(("inventory-debug", sub_matches)) => {
            let inventory = load_inventory(sub_matches, &config)?;

            println!("\n=== Inventory Debug Information ===");
            println!("Total hosts: {}", inventory.hosts.len());
            println!("Total groups: {}", inventory.groups.len());

            println!("\n== Groups ==");
            let mut groups: Vec<_> = inventory.groups.iter().collect();
            groups.sort_by_key(|(name, _)| *name);
            for (name, group) in groups {
                println!("Group: {} ({} hosts)", name, group.hosts.len());
                if !group.hosts.is_empty() {
                    println!("  Hosts:");
                    let mut host_names: Vec<_> = group.hosts.iter().collect();
                    host_names.sort();
                    for host_name in host_names {
                        if let Some(host) = inventory.hosts.get(host_name) {
                            let config = rustsible::ssh::config::ConnectionConfig::from_host(host)?;
                            println!(
                                "    - {} ({}:{})",
                                host.name,
                                config.hostname(),
                                config.port()
                            );
                        } else {
                            println!("    - {} (NOT FOUND IN INVENTORY)", host_name);
                        }
                    }
                }

                if !group.variables.is_empty() {
                    println!("  Variables:");
                    let mut variables: Vec<_> = group.variables.iter().collect();
                    variables.sort_by_key(|(key, _)| *key);
                    for (key, value) in variables {
                        println!("    - {} = {}", key, display_inventory_value(key, value));
                    }
                }
            }

            println!("\n== Hosts ==");
            let mut hosts: Vec<_> = inventory.hosts.iter().collect();
            hosts.sort_by_key(|(name, _)| *name);
            for (name, host) in hosts {
                let config = rustsible::ssh::config::ConnectionConfig::from_host(host)?;
                println!("Host: {} ({}:{})", name, config.hostname(), config.port());
                if !host.variables.is_empty() {
                    println!("  Variables:");
                    let mut variables: Vec<_> = host.variables.iter().collect();
                    variables.sort_by_key(|(key, _)| *key);
                    for (key, value) in variables {
                        println!("    - {} = {}", key, display_inventory_value(key, value));
                    }
                }
            }
        }
        _ => {
            eprintln!("Unknown command");
            std::process::exit(1);
        }
    }

    Ok(())
}

fn load_inventory(
    matches: &clap::ArgMatches,
    config: &rustsible::config::Config,
) -> Result<inventory::Inventory> {
    let path = matches
        .get_one::<String>("inventory")
        .map(std::path::PathBuf::from)
        .or_else(|| config.inventory.clone())
        .unwrap_or_else(|| "inventory".into());
    let mut inventory = inventory::parse(
        path.to_str()
            .ok_or_else(|| anyhow::anyhow!("Inventory path must be UTF-8"))?,
    )?;
    config.apply_inventory(&mut inventory);
    Ok(inventory)
}

fn required_cli_string<'a>(value: Option<&'a String>, description: &str) -> Result<&'a str> {
    value.map(String::as_str).ok_or_else(|| {
        anyhow::anyhow!(
            "Command-line parser did not provide the required {}",
            description
        )
    })
}

fn credentials(matches: &clap::ArgMatches) -> Result<rustsible::ssh::credentials::Credentials> {
    rustsible::ssh::credentials::Credentials::read(
        matches.get_flag("ask-pass"),
        matches.get_flag("ask-become-pass"),
    )
}

#[cfg(test)]
fn required_cli_usize(value: Option<&usize>, description: &str) -> Result<usize> {
    value.copied().ok_or_else(|| {
        anyhow::anyhow!(
            "Command-line parser did not provide the required {} value",
            description
        )
    })
}

fn display_inventory_value(_key: &str, _value: impl std::fmt::Debug) -> &'static str {
    // Inventory values are untrusted secret-bearing input. A deny-list can
    // never cover custom names such as `github_pat` or `session_id`, so the
    // diagnostic command deliberately shows keys but never raw values.
    "<redacted>"
}

#[cfg(test)]
mod tests {
    use super::{display_inventory_value, required_cli_string, required_cli_usize};

    #[test]
    fn inventory_debug_redacts_credentials() {
        assert_eq!(
            display_inventory_value("ansible_password", "hunter2"),
            "<redacted>"
        );
        assert_eq!(display_inventory_value("api_token", "abcdef"), "<redacted>");
        assert_eq!(display_inventory_value("api_key", "abcdef"), "<redacted>");
        assert_eq!(
            display_inventory_value("authorization", "bearer"),
            "<redacted>"
        );
        assert_eq!(
            display_inventory_value("session_cookie", "abcdef"),
            "<redacted>"
        );
        assert_eq!(display_inventory_value("region", "cn-east"), "<redacted>");
        assert_eq!(
            display_inventory_value("github_pat", "abcdef"),
            "<redacted>"
        );
    }

    #[test]
    fn missing_required_cli_values_are_diagnostic_errors() {
        assert!(required_cli_string(None, "module name")
            .unwrap_err()
            .to_string()
            .contains("module name"));
        assert!(required_cli_usize(None, "--forks")
            .unwrap_err()
            .to_string()
            .contains("--forks"));

        let module = "debug".to_string();
        assert_eq!(
            required_cli_string(Some(&module), "module").unwrap(),
            "debug"
        );
        assert_eq!(required_cli_usize(Some(&5), "--forks").unwrap(), 5);
    }
}
