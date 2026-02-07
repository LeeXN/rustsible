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

    info!("Starting Rustsible - Ansible-compatible automation in Rust");

    match matches.subcommand() {
        Some(("playbook", sub_matches)) => {
            let playbook_file = sub_matches.get_one::<String>("playbook").unwrap();
            let inventory_file = sub_matches
                .get_one::<String>("inventory")
                .map(|s| s.as_str())
                .unwrap_or("inventory");

            info!("Running playbook: {}", playbook_file);
            let inventory = inventory::parse(inventory_file)?;
            let result = playbook::execute(playbook_file, &inventory);

            if let Err(e) = result {
                eprintln!("Error executing playbook: {}", e);
                std::process::exit(1);
            }
        }
        Some(("ad-hoc", sub_matches)) => {
            let module = sub_matches.get_one::<String>("module").unwrap();
            let args = sub_matches.get_one::<String>("args").unwrap();
            let host_pattern = sub_matches.get_one::<String>("pattern").unwrap();
            let inventory_file = sub_matches
                .get_one::<String>("inventory")
                .map(|s| s.as_str())
                .unwrap_or("inventory");

            info!("Running ad-hoc command with module: {}", module);
            let inventory = inventory::parse(inventory_file)?;
            let hosts = inventory.filter_hosts(host_pattern);

            if hosts.is_empty() {
                eprintln!("No hosts matched the pattern: {}", host_pattern);
                std::process::exit(1);
            }

            let result = modules::run_adhoc(&hosts, module, args);
            if let Err(e) = result {
                eprintln!("Error executing ad-hoc command: {}", e);
                std::process::exit(1);
            }
        }
        Some(("inventory-debug", sub_matches)) => {
            let inventory_file = sub_matches
                .get_one::<String>("inventory")
                .map(|s| s.as_str())
                .unwrap_or("inventory");

            info!("Debugging inventory file: {}", inventory_file);
            let inventory = inventory::parse(inventory_file)?;

            println!("\n=== Inventory Debug Information ===");
            println!("Total hosts: {}", inventory.hosts.len());
            println!("Total groups: {}", inventory.groups.len());

            println!("\n== Groups ==");
            for (name, group) in &inventory.groups {
                println!("Group: {} ({} hosts)", name, group.hosts.len());
                if !group.hosts.is_empty() {
                    println!("  Hosts:");
                    for host_name in &group.hosts {
                        if let Some(host) = inventory.hosts.get(host_name) {
                            println!("    - {} ({}:{})", host.name, host.hostname, host.port);
                        } else {
                            println!("    - {} (NOT FOUND IN INVENTORY)", host_name);
                        }
                    }
                }

                if !group.variables.is_empty() {
                    println!("  Variables:");
                    for (key, value) in &group.variables {
                        println!("    - {} = {}", key, value);
                    }
                }
            }

            println!("\n== Hosts ==");
            for (name, host) in &inventory.hosts {
                println!("Host: {} ({}:{})", name, host.hostname, host.port);
                if !host.variables.is_empty() {
                    println!("  Variables:");
                    for (key, value) in &host.variables {
                        println!("    - {} = {}", key, value);
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
