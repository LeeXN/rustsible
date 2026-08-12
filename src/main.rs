use rustsible::{cli, inventory, modules, playbook};

use anyhow::Result;
use env_logger::Builder;
use log::{info, LevelFilter};
use std::io::Write;

fn main() -> Result<()> {
    // Delay logger initialization until after parsing arguments
    let app = cli::build_cli();
    let matches = app.get_matches();

    // Set log level based on the number of verbose flags
    let log_level = match matches.subcommand() {
        Some((_, sub_matches)) => match sub_matches.get_count("verbose") {
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

    info!("Starting Rustsible");

    match matches.subcommand() {
        Some(("playbook", sub_matches)) => {
            let playbook_file =
                required_cli_string(sub_matches.get_one::<String>("playbook"), "playbook file")?;
            let inventory_file = sub_matches
                .get_one::<String>("inventory")
                .map_or("inventory", String::as_str);

            info!("Running playbook: {}", playbook_file);
            let inventory = inventory::parse(inventory_file)?;
            let forks = required_cli_usize(sub_matches.get_one::<usize>("forks"), "--forks")?;
            if forks == 0 {
                return Err(anyhow::anyhow!("--forks must be greater than zero"));
            }
            let options = playbook::ExecutionOptions {
                limit: sub_matches.get_one::<String>("limit").cloned(),
                check_mode: sub_matches.get_flag("check"),
                forks,
            };
            let result = playbook::execute_with_options(playbook_file, &inventory, &options);

            if let Err(e) = result {
                eprintln!("Error executing playbook: {}", e);
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
            let inventory_file = sub_matches
                .get_one::<String>("inventory")
                .map_or("inventory", String::as_str);

            info!("Running ad-hoc command with module: {}", module);
            let inventory = inventory::parse(inventory_file)?;
            let hosts = inventory.filter_hosts(host_pattern);

            if hosts.is_empty() {
                eprintln!("No hosts matched the pattern: {}", host_pattern);
                std::process::exit(1);
            }

            let forks = required_cli_usize(sub_matches.get_one::<usize>("forks"), "--forks")?;
            if forks == 0 {
                return Err(anyhow::anyhow!("--forks must be greater than zero"));
            }
            let options = modules::AdHocOptions {
                // Omitting --become deliberately leaves the decision to each
                // host's inventory. --become-user selects the target account
                // but, like Ansible, does not itself enable escalation.
                become_override: sub_matches.get_flag("become").then_some(true),
                become_user: sub_matches.get_one::<String>("become-user").cloned(),
                check_mode: sub_matches.get_flag("check"),
                forks,
            };
            let result = modules::run_adhoc_with_options(&hosts, module, args, &options);
            if let Err(e) = result {
                eprintln!("Error executing ad-hoc command: {}", e);
                std::process::exit(1);
            }
        }
        Some(("inventory-debug", sub_matches)) => {
            let inventory_file = sub_matches
                .get_one::<String>("inventory")
                .map_or("inventory", String::as_str);

            info!("Debugging inventory file: {}", inventory_file);
            let inventory = inventory::parse(inventory_file)?;

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
                            println!("    - {} ({}:{})", host.name, host.hostname, host.port);
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
                println!("Host: {} ({}:{})", name, host.hostname, host.port);
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

fn required_cli_string<'a>(value: Option<&'a String>, description: &str) -> Result<&'a str> {
    value.map(String::as_str).ok_or_else(|| {
        anyhow::anyhow!(
            "Command-line parser did not provide the required {}",
            description
        )
    })
}

fn required_cli_usize(value: Option<&usize>, description: &str) -> Result<usize> {
    value.copied().ok_or_else(|| {
        anyhow::anyhow!(
            "Command-line parser did not provide the required {} value",
            description
        )
    })
}

fn display_inventory_value(_key: &str, _value: &str) -> &'static str {
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
