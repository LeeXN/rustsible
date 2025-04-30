pub mod command;
pub mod shell;
pub mod copy;
pub mod file;
pub mod template;
pub mod service;
pub mod package;
pub mod debug;
pub mod local;

use anyhow::Result;
use serde_yaml::Value;
use log::info;
use std::time::Instant;
use std::collections::HashMap;
use colored::Colorize;

use crate::inventory::Host;

/// Result structure for unified handling of module returns
#[derive(Default)]
pub struct ModuleResult {
    pub stdout: String,
    pub stderr: String, 
    pub changed: bool,
    pub msg: String,
}

/// Run an ad-hoc command on a list of hosts
pub fn run_adhoc(hosts: &[Host], module_name: &str, args: &str) -> Result<()> {
    info!("Running ad-hoc module '{}' on {} hosts", module_name, hosts.len());
    
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
            },
            "shell" => {
                let value = Value::String(args.to_string());
                shell::execute_adhoc(host, &value)
            },
            "copy" => {
                // Parse args in the format "src=file dest=path"
                let params = parse_args(args)?;
                let value = Value::Mapping(params);
                copy::execute_adhoc(host, &value)
            },
            "file" => {
                // Parse args in the format "path=file state=absent"
                let params = parse_args(args)?;
                let value = Value::Mapping(params);
                file::execute_adhoc(host, &value)
            },
            "template" => {
                // Parse args in the format "src=file dest=path"
                let params = parse_args(args)?;
                let value = Value::Mapping(params);
                template::execute_adhoc(host, &value)
            },
            "service" => {
                // Parse args in the format "name=service state=started"
                let params = parse_args(args)?;
                let value = Value::Mapping(params);
                service::execute_adhoc(host, &value)
            },
            "package" => {
                // Parse args in the format "name=package state=present"
                let params = parse_args(args)?;
                let value = Value::Mapping(params);
                package::execute_adhoc(host, &value)
            },
            "debug" => {
                // Parse args in the format "name=package state=present"
                let params = parse_args(args)?;
                let value = Value::Mapping(params);
                debug::execute_adhoc(host, &value)
            },
            _ => {
                Err(anyhow::anyhow!("Unknown module: {}", module_name))
            }
        };
        
        let elapsed = start_time.elapsed();
        let execution_time = format!("{:.2}s", elapsed.as_secs_f64());
        
        if let Ok(module_result) = result {
            success_count += 1;
            print_host_result(&host.name, true, &execution_time, &module_result, None);
            results.insert(host.name.clone(), module_result);
        } else if let Err(e) = result {
            failed_hosts.push(host.name.clone());
            print_host_result(&host.name, false, &execution_time, &ModuleResult::default(), Some(&e.to_string()));
        }
    }
    
    // Output result summary
    print_summary(hosts.len(), success_count, &failed_hosts);
    
    // Return success if all hosts were successful
    if failed_hosts.is_empty() {
        Ok(())
    } else {
        // Return error if any host failed, but don't interrupt execution of other hosts
        Err(anyhow::anyhow!("Task failed on {} of {} hosts", failed_hosts.len(), hosts.len()))
    }
}

/// Print execution result for a single host
fn print_host_result(host_name: &str, success: bool, execution_time: &str, 
                    result: &ModuleResult, error_msg: Option<&str>) {
    let host_info = format!("{:<15}", host_name);
    let status = if result.changed { "changed" } else { "ok" };
    
    if success {
        let status_color = if result.changed { 
            status.yellow().bold() 
        } else { 
            status.green().bold() 
        };
        println!("{} : {} => ({})", host_info.green(), status_color, execution_time.dimmed());
        
        if !result.msg.is_empty() {
            println!("  {}", result.msg);
        }
        
        // Display command output
        if !result.stdout.is_empty() {
            println!("  stdout:");
            for line in result.stdout.lines() {
                println!("    {}", line);
            }
        }
        
        if !result.stderr.is_empty() {
            println!("  stderr:");
            for line in result.stderr.lines() {
                println!("    {}", line.red());
            }
        }
    } else {
        println!("{} : {} => ({})", host_info.red(), "failed".red().bold(), execution_time.dimmed());
        if let Some(error) = error_msg {
            println!("  {}", error.red());
        }
    }
}

/// Print task execution summary
fn print_summary(total: usize, success: usize, failed_hosts: &[String]) {
    println!("\n{}", "PLAY RECAP".bold());
    println!("{}", "----------".dimmed());
    
    if failed_hosts.is_empty() {
        println!("{}: {}/{} hosts succeeded", "ok".green().bold(), success, total);
    } else {
        println!("{}: {}/{} hosts succeeded", "ok".green().bold(), success, total);
        println!("{}: {}/{} hosts failed", "failed".red().bold(), failed_hosts.len(), total);
        println!("\nFailed hosts:");
        for host in failed_hosts {
            println!("  {}", host.red());
        }
    }
}

/// Parse command-line arguments in the format "key1=value1 key2=value2"
fn parse_args(args: &str) -> Result<serde_yaml::Mapping> {
    let mut params = serde_yaml::Mapping::new();
    
    for part in args.split_whitespace() {
        if let Some((key, value)) = part.split_once('=') {
            params.insert(
                Value::String(key.to_string()), 
                Value::String(value.to_string())
            );
        } else {
            return Err(anyhow::anyhow!("Invalid argument format: {}", part));
        }
    }
    
    Ok(params)
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