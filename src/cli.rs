use clap::{Arg, ArgAction, Command};

pub fn build_cli() -> Command {
    Command::new("rustsible")
        .about("An Ansible-inspired automation tool written in Rust")
        .version(env!("CARGO_PKG_VERSION"))
        .subcommand_required(true)
        .arg_required_else_help(true)
        .args(vault_args())
        .subcommand(vault_command())
        .arg(
            Arg::new("config")
                .long("config")
                .global(true)
                .value_name("FILE")
                .value_parser(clap::value_parser!(std::path::PathBuf))
                .help("Explicit ansible.cfg file"),
        )
        .arg(
            Arg::new("worker-dir")
                .long("worker-dir")
                .global(true)
                .help("Directory containing the release worker manifest and binaries")
                .value_name("DIRECTORY")
                .value_parser(clap::value_parser!(std::path::PathBuf)),
        )
        .subcommand(
            Command::new("playbook").args(vault_args())
                .arg(extra_vars_arg())
                .args(connection_args())
                .arg(Arg::new("diff").long("diff").action(ArgAction::SetTrue).help("Show bounded structured differences from supported modules"))
                .arg(Arg::new("tags").long("tags").action(ArgAction::Append).value_name("TAGS").help("Run tasks matching comma-separated tags"))
                .arg(Arg::new("skip-tags").long("skip-tags").action(ArgAction::Append).value_name("TAGS").help("Skip tasks matching comma-separated tags"))
                .arg(Arg::new("syntax-check").long("syntax-check").action(ArgAction::SetTrue).conflicts_with_all(["list-hosts", "list-tasks"]).help("Validate static playbook structure without connecting"))
                .arg(Arg::new("list-hosts").long("list-hosts").action(ArgAction::SetTrue).conflicts_with("list-tasks").help("List selected hosts as JSON without connecting"))
                .arg(Arg::new("list-tasks").long("list-tasks").action(ArgAction::SetTrue).help("List selected static tasks as JSON without connecting"))
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
            Command::new("ad-hoc").args(vault_args())
                .arg(extra_vars_arg())
                .args(connection_args())
                .arg(Arg::new("diff").long("diff").action(ArgAction::SetTrue).help("Show bounded structured differences from supported modules"))
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
                        .default_value("")
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
                    Arg::new("background").short('B').long("background")
                        .value_name("SECONDS").value_parser(clap::value_parser!(u64)).default_value("0")
                        .help("Maximum runtime for a native background task; zero runs synchronously"),
                )
                .arg(
                    Arg::new("poll").short('P').long("poll")
                        .value_name("SECONDS").value_parser(clap::value_parser!(u64)).default_value("15")
                        .help("Poll interval for a background task; zero returns its job ID immediately"),
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
            Command::new("inventory-debug").args(vault_args())
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

fn extra_vars_arg() -> Arg {
    Arg::new("extra-vars")
        .short('e')
        .long("extra-vars")
        .action(ArgAction::Append)
        .value_name("VARS")
        .help("Extra variables as name=value, a YAML/JSON mapping, or @file")
}

fn vault_args() -> [Arg; 3] {
    [
        Arg::new("vault-password-file")
            .long("vault-password-file")
            .action(ArgAction::Append)
            .value_parser(clap::value_parser!(std::path::PathBuf))
            .value_name("FILE")
            .help("Read a Vault password from a regular non-executable file"),
        Arg::new("vault-id")
            .long("vault-id")
            .action(ArgAction::Append)
            .value_name("ID@SOURCE")
            .help("Vault password source: ID@FILE or ID@prompt"),
        Arg::new("ask-vault-pass")
            .long("ask-vault-pass")
            .action(ArgAction::SetTrue)
            .help("Prompt for a Vault password on the controlling terminal"),
    ]
}
fn vault_command() -> Command {
    let mut command = Command::new("vault")
        .args(vault_args())
        .about("Encrypt, decrypt, view or rekey Ansible Vault files")
        .subcommand_required(true);
    for action in ["encrypt", "decrypt", "view", "rekey"] {
        let mut sub = Command::new(action).args(vault_args()).arg(
            Arg::new("input")
                .value_parser(clap::value_parser!(std::path::PathBuf))
                .value_name("FILE")
                .required(true)
                .index(1),
        );
        if action != "view" {
            sub = sub.arg(
                Arg::new("output")
                    .long("output")
                    .value_parser(clap::value_parser!(std::path::PathBuf))
                    .value_name("FILE")
                    .help("Output file; '-' writes stdout (default: replace input)"),
            );
        }
        if matches!(action, "encrypt" | "rekey") {
            sub = sub.arg(
                Arg::new("encrypt-vault-id")
                    .long("encrypt-vault-id")
                    .value_name("ID"),
            );
        }
        if action == "rekey" {
            sub = sub
                .arg(
                    Arg::new("new-vault-password-file")
                        .long("new-vault-password-file")
                        .action(ArgAction::Append)
                        .value_parser(clap::value_parser!(std::path::PathBuf))
                        .value_name("FILE"),
                )
                .arg(
                    Arg::new("new-vault-id")
                        .long("new-vault-id")
                        .action(ArgAction::Append)
                        .value_name("ID@SOURCE"),
                );
        }
        command = command.subcommand(sub);
    }
    command
}

fn connection_args() -> [Arg; 3] {
    [
        Arg::new("ask-pass")
            .short('k')
            .long("ask-pass")
            .action(ArgAction::SetTrue)
            .help("Prompt for SSH password on the controlling terminal"),
        Arg::new("ask-become-pass")
            .short('K')
            .long("ask-become-pass")
            .action(ArgAction::SetTrue)
            .help("Prompt for privilege escalation password on the controlling terminal"),
        Arg::new("private-key")
            .long("private-key")
            .value_parser(clap::value_parser!(std::path::PathBuf))
            .value_name("FILE")
            .help("SSH private key file"),
    ]
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

    #[test]
    fn setup_ad_hoc_does_not_require_an_args_flag() {
        let matches = build_cli()
            .try_get_matches_from(["rustsible", "ad-hoc", "all", "--module", "setup"])
            .unwrap();
        let (_, ad_hoc) = matches.subcommand().unwrap();
        assert_eq!(
            ad_hoc.get_one::<String>("args").map(String::as_str),
            Some("")
        );
    }
}
