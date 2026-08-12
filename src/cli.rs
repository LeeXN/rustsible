use clap::{Arg, ArgAction, Command};

pub fn build_cli() -> Command {
    Command::new("rustsible")
        .about("An Ansible-inspired automation tool written in Rust")
        .version(env!("CARGO_PKG_VERSION"))
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(
            Command::new("playbook")
                .about("Run a Rustsible playbook")
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
                )
                .arg(
                    Arg::new("forks")
                        .short('f')
                        .long("forks")
                        .help("Maximum number of hosts processed in parallel")
                        .value_name("FORKS")
                        .value_parser(clap::value_parser!(usize))
                        .default_value("5"),
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
                )
                .arg(
                    Arg::new("become")
                        .short('b')
                        .long("become")
                        .help("Run operations with privilege escalation")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("become-user")
                        .long("become-user")
                        .help("Run operations as this user (default: root)")
                        .value_name("USER"),
                )
                .arg(
                    Arg::new("check")
                        .long("check")
                        .help("Predict changes without applying them")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("forks")
                        .short('f')
                        .long("forks")
                        .help("Maximum number of hosts processed in parallel")
                        .value_name("FORKS")
                        .value_parser(clap::value_parser!(usize))
                        .default_value("5"),
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

    #[test]
    fn ad_hoc_accepts_execution_control_options() {
        let matches = build_cli()
            .try_get_matches_from([
                "rustsible",
                "ad-hoc",
                "web",
                "--module",
                "file",
                "--args",
                "path=/tmp/demo state=touch",
                "--become",
                "--become-user",
                "deploy",
                "--check",
                "--forks",
                "7",
            ])
            .unwrap();
        let (_, ad_hoc) = matches.subcommand().unwrap();

        assert!(ad_hoc.get_flag("become"));
        assert_eq!(
            ad_hoc.get_one::<String>("become-user").map(String::as_str),
            Some("deploy")
        );
        assert!(ad_hoc.get_flag("check"));
        assert_eq!(ad_hoc.get_one::<usize>("forks"), Some(&7));
    }
}
