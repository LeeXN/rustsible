use clap::{Arg, ArgAction, Command};

pub fn build_cli() -> Command {
    Command::new("rustsible")
        .about("Ansible-compatible IT automation tool written in Rust")
        .version(env!("CARGO_PKG_VERSION"))
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(
            Command::new("playbook")
                .about("Run an Ansible-compatible playbook")
                .arg(
                    Arg::new("playbook")
                        .help("Playbook file to run")
                        .required(true)
                        .index(1),
                )
                .arg(
                    Arg::new("inventory")
                        .short('i')
                        .long("inventory")
                        .help("Specify inventory file path (default: 'inventory')")
                        .value_name("INVENTORY"),
                )
                .arg(
                    Arg::new("verbose")
                        .short('v')
                        .action(ArgAction::Count)
                        .help("Increase verbosity (up to -vvvv)"),
                )
                .arg(
                    Arg::new("limit")
                        .short('l')
                        .long("limit")
                        .help("Limit to specified hosts or groups")
                        .value_name("SUBSET"),
                )
                .arg(
                    Arg::new("check")
                        .long("check")
                        .help("Perform a dry run without making changes")
                        .action(ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("ad-hoc")
                .about("Run an ad-hoc command on managed nodes")
                .arg(
                    Arg::new("pattern")
                        .help("Host pattern to target")
                        .required(true)
                        .index(1),
                )
                .arg(
                    Arg::new("module")
                        .short('m')
                        .long("module")
                        .help("Module name to execute")
                        .required(true)
                        .value_name("MODULE"),
                )
                .arg(
                    Arg::new("args")
                        .short('a')
                        .long("args")
                        .help("Module arguments")
                        .required(true)
                        .value_name("ARGS"),
                )
                .arg(
                    Arg::new("inventory")
                        .short('i')
                        .long("inventory")
                        .help("Specify inventory file path (default: 'inventory')")
                        .value_name("INVENTORY"),
                )
                .arg(
                    Arg::new("verbose")
                        .short('v')
                        .action(ArgAction::Count)
                        .help("Increase verbosity (up to -vvvv)"),
                ),
        )
        .subcommand(
            Command::new("inventory-debug")
                .about("Debug and display information about inventory files")
                .arg(
                    Arg::new("inventory")
                        .short('i')
                        .long("inventory")
                        .help("Specify inventory file path (default: 'inventory')")
                        .value_name("INVENTORY"),
                )
                .arg(
                    Arg::new("verbose")
                        .short('v')
                        .action(ArgAction::Count)
                        .help("Increase verbosity (up to -vvvv)"),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::build_cli;

    #[test]
    fn test_build_cli_subcommands() {
        let cmd = build_cli();
        let subcommands: Vec<_> = cmd.get_subcommands().map(|s| s.get_name()).collect();
        assert!(subcommands.contains(&"playbook"));
        assert!(subcommands.contains(&"ad-hoc"));
        assert!(subcommands.contains(&"inventory-debug"));
    }
}
