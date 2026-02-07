pub mod command;
pub mod copy;
pub mod debug;
pub mod file;
pub mod lineinfile;
pub mod local;
pub mod package;
pub mod param;
pub mod remote;
pub mod service;
pub mod shell;
pub mod template;
pub mod user;

use anyhow::Result;
use colored::Colorize;
use log::info;
use serde_yaml::Value;
use std::collections::HashMap;
use std::time::Instant;

use crate::inventory::Host;
use crate::ssh::connection::SshClient;

/// Result structure for unified handling of module returns
#[derive(Default)]
pub struct ModuleResult {
    pub stdout: String,
    pub stderr: String,
    pub changed: bool,
    pub failed: bool,
    pub msg: String,
}

/// Trait for common module execution patterns
pub trait ModuleExecutor {
    /// Execute the module with the given SSH client and arguments
    fn execute(
        ssh_client: &SshClient,
        args: &Value,
        use_become: bool,
        become_user: &str,
    ) -> Result<ModuleResult>;

    /// Execute the module in ad-hoc mode for a single host
    fn execute_adhoc(host: &Host, args: &Value) -> Result<ModuleResult> {
        info!("Connecting to host: {}", host.name);
        let ssh_client = SshClient::connect(host)?;
        Self::execute(&ssh_client, args, false, "")
    }

    /// Helper to execute a command on a remote host with proper sudo handling
    fn execute_command(
        ssh_client: &SshClient,
        cmd: &str,
        use_become: bool,
        become_user: &str,
    ) -> Result<(i32, String, String)> {
        info!("Executing command: {}", cmd);

        if use_become {
            ssh_client.execute_sudo_command(cmd, become_user)
        } else {
            ssh_client.execute_command(cmd)
        }
    }

    /// Helper to process command execution results into a ModuleResult
    fn process_command_result(
        exit_code: i32,
        stdout: String,
        stderr: String,
        success_msg: &str,
        error_prefix: &str,
    ) -> Result<ModuleResult> {
        let module_result = ModuleResult {
            stdout,
            stderr: stderr.clone(),
            changed: true,
            failed: false,
            msg: format!("{} (exit code: {})", success_msg, exit_code),
        };

        if exit_code != 0 {
            let error_msg = if stderr.trim().is_empty() {
                format!("{} (exit code: {})", error_prefix, exit_code)
            } else {
                format!(
                    "{} (exit code: {}): {}",
                    error_prefix,
                    exit_code,
                    stderr.trim()
                )
            };
            return Err(anyhow::anyhow!(error_msg));
        }

        info!("{}", success_msg);
        Ok(module_result)
    }

    /// Helper to extract a string argument from Value, handling both String and Mapping with "cmd" key
    fn extract_command_arg(args: &Value) -> Result<String> {
        match args {
            Value::String(cmd) => Ok(cmd.clone()),
            Value::Mapping(map) => {
                if let Some(Value::String(cmd)) = map.get(&Value::String("cmd".to_string())) {
                    Ok(cmd.clone())
                } else {
                    Err(anyhow::anyhow!("Module requires a valid command string"))
                }
            }
            _ => Err(anyhow::anyhow!("Module requires a valid command string")),
        }
    }
}

/// Run an ad-hoc command on a list of hosts
pub fn run_adhoc(hosts: &[Host], module_name: &str, args: &str) -> Result<()> {
    info!(
        "Running ad-hoc module '{}' on {} hosts",
        module_name,
        hosts.len()
    );

    let mut success_count = 0;
    let mut failed_hosts = Vec::new();
    let mut results: HashMap<String, ModuleResult> = HashMap::new();

    println!("\n{}", "TASK [Execute ad-hoc command]".bold());
    println!("{}\n", "------------------------------".dimmed());

    for host in hosts {
        info!("Running module {} on host {}", module_name, host.name);
        let start_time = Instant::now();

        let result = match module_name {
            "command" => {
                let value = Value::String(args.to_string());
                command::execute_adhoc(host, &value)
            }
            "shell" => {
                let value = Value::String(args.to_string());
                shell::execute_adhoc(host, &value)
            }
            "copy" => {
                // Parse args in the format "src=file dest=path"
                let params = parse_args(args)?;
                let value = Value::Mapping(params);
                copy::execute_adhoc(host, &value)
            }
            "file" => {
                // Parse args in the format "path=file state=absent"
                let params = parse_args(args)?;
                let value = Value::Mapping(params);
                file::execute_adhoc(host, &value)
            }
            "template" => {
                // Parse args in the format "src=file dest=path"
                let params = parse_args(args)?;
                let value = Value::Mapping(params);
                template::execute_adhoc(host, &value)
            }
            "service" => {
                // Parse args in the format "name=service state=started"
                let params = parse_args(args)?;
                let value = Value::Mapping(params);
                service::execute_adhoc(host, &value)
            }
            "package" => {
                // Parse args in the format "name=package state=present"
                let params = parse_args(args)?;
                let value = Value::Mapping(params);
                package::execute_adhoc(host, &value)
            }
            "debug" => {
                // Parse args in the format "name=package state=present"
                let params = parse_args(args)?;
                let value = Value::Mapping(params);
                debug::execute_adhoc(host, &value)
            }
            "lineinfile" => {
                // Parse args in the format "path=file state=absent"
                let params = parse_args(args)?;
                let value = Value::Mapping(params);
                lineinfile::execute_adhoc(host, &value)
            }
            "user" => {
                // Parse args in the format "name=user state=present"
                let params = parse_args(args)?;
                let value = Value::Mapping(params);
                user::execute_adhoc(host, &value)
            }
            _ => {
                return Err(anyhow::anyhow!("Unsupported module: {}", module_name));
            }
        };

        match result {
            Ok(module_result) => {
                success_count += 1;
                println!(
                    "{} | {} | rc={} >>>\n{}",
                    host.name.green(),
                    "SUCCESS".green(),
                    0,
                    if !module_result.stdout.trim().is_empty() {
                        module_result.stdout.trim()
                    } else {
                        module_result.msg.as_str()
                    }
                );
                results.insert(host.name.clone(), module_result);
            }
            Err(e) => {
                failed_hosts.push(host.name.clone());
                println!("{} | {} | rc=1 >>>\n{}", host.name.red(), "FAILED".red(), e);
            }
        }

        let duration = start_time.elapsed();
        println!(
            "\n{}\n",
            format!("Execution time: {:.2?}", duration).dimmed()
        );
    }

    println!("\n{}", "PLAY RECAP".bold());
    println!("{}\n", "----------".dimmed());

    for host in hosts {
        if failed_hosts.contains(&host.name) {
            println!(
                "{}: {}={} {}={} {}={}",
                host.name.bold(),
                "ok".green(),
                0,
                "changed".yellow(),
                0,
                "failed".red(),
                1
            );
        } else if let Some(result) = results.get(&host.name) {
            println!(
                "{}: {}={} {}={} {}={}",
                host.name.bold(),
                "ok".green(),
                1,
                "changed".yellow(),
                if result.changed { 1 } else { 0 },
                "failed".red(),
                0
            );
        }
    }

    println!(
        "\n{}: {}/{}",
        "SUCCESS RATE".bold(),
        success_count.to_string().green(),
        hosts.len()
    );

    if !failed_hosts.is_empty() {
        return Err(anyhow::anyhow!(
            "Failed to execute on {} hosts: {}",
            failed_hosts.len(),
            failed_hosts.join(", ")
        ));
    }

    Ok(())
}

/// Parse command line arguments in format "key1=value1 key2=value2"
fn parse_args(args_str: &str) -> Result<serde_yaml::Mapping> {
    let mut mapping = serde_yaml::Mapping::new();
    for part in args_str.split_whitespace() {
        let mut kv = part.splitn(2, '=');
        if let (Some(key), Some(value)) = (kv.next(), kv.next()) {
            // Try to parse as different types
            let parsed_value = if value.eq_ignore_ascii_case("true") {
                Value::Bool(true)
            } else if value.eq_ignore_ascii_case("false") {
                Value::Bool(false)
            } else if let Ok(num) = value.parse::<i64>() {
                Value::Number(serde_yaml::Number::from(num))
            } else if let Ok(num) = value.parse::<f64>() {
                Value::Number(serde_yaml::Number::from(num))
            } else {
                Value::String(value.to_string())
            };

            mapping.insert(Value::String(key.to_string()), parsed_value);
        } else {
            return Err(anyhow::anyhow!("Invalid argument format: {}", part));
        }
    }
    Ok(mapping)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::Value;

    #[test]
    fn test_module_result_default() {
        let result = ModuleResult::default();
        assert_eq!(result.stdout, "");
        assert_eq!(result.stderr, "");
        assert_eq!(result.changed, false);
        assert_eq!(result.msg, "");
    }

    #[test]
    fn test_parse_args() {
        // Test normal parameter parsing
        let args = "key1=value1 key2=value2";
        let mapping = parse_args(args).unwrap();

        assert_eq!(
            mapping.get(&Value::String("key1".to_string())),
            Some(&Value::String("value1".to_string()))
        );

        assert_eq!(
            mapping.get(&Value::String("key2".to_string())),
            Some(&Value::String("value2".to_string()))
        );

        // Test invalid parameters
        let invalid_args = "invalid_format";
        assert!(parse_args(invalid_args).is_err());
    }
}
