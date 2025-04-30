use anyhow::{Result, anyhow};
use serde_yaml::Value;
use std::process::Command;
use log::{info, debug};

use crate::modules::ModuleResult;
use crate::ssh::connection::SshClient;
use crate::inventory::Host;

/// Execute the command module locally (without SSH)
pub fn execute_local_command(command: &str) -> Result<(i32, String, String)> {
    debug!("Executing local command: {}", command);
    
    // Use shell for more consistent behavior across platforms
    let output = if cfg!(target_os = "windows") {
        Command::new("cmd")
            .args(&["/C", command])
            .output()
    } else {
        Command::new("sh")
            .args(&["-c", command])
            .output()
    }.map_err(|e| anyhow!("Failed to execute command: {}", e))?;
    
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    
    Ok((output.status.code().unwrap_or(1), stdout, stderr))
}

/// Execute the command module - runs a command on the target host
pub fn execute(
    ssh_client: &SshClient, 
    args: &Value, 
    use_become: bool, 
    become_user: &str
) -> Result<ModuleResult> {
    if let Value::Mapping(map) = args {
        // Get the raw command to execute
        let cmd = match map.get(&Value::String("_raw_params".to_string())) {
            Some(Value::String(cmd)) => cmd,
            _ => return Err(anyhow!("Command module requires a command string")),
        };

        // Check if this is a local execution 
        let host_type = map.get(&Value::String("_host_type".to_string()));
        let is_local = match host_type {
            Some(Value::String(host_type)) => host_type == "local",
            _ => false, 
        };

        let (exit_code, stdout, stderr) = if is_local {
            // Run command locally
            execute_local_command(cmd)?
        } else if use_become {
            // Run command remotely with sudo
            ssh_client.execute_sudo_command(cmd, become_user)?
        } else {
            // Run command remotely without sudo
            ssh_client.execute_command(cmd)?
        };
        
        let mut result = ModuleResult::default();
        result.stdout = stdout;
        result.stderr = stderr;
        
        if exit_code == 0 {
            result.changed = true;
            result.msg = format!("Command executed successfully (exit code: {})", exit_code);
        } else {
            result.msg = format!("Command failed with exit code: {}", exit_code);
        }
        
        Ok(result)
    } else {
        Err(anyhow!("Command module requires parameters"))
    }
}

/// Execute the command module for ad-hoc commands
#[allow(dead_code)]
pub fn execute_adhoc(host: &Host, args: &Value) -> Result<ModuleResult> {
    if host.hostname == "localhost" || host.hostname == "127.0.0.1" {
        // For localhost, execute directly without SSH
        return execute_adhoc_local(host, args);
    }
    
    info!("Connecting to host: {}", host.name);
    let ssh_client = match crate::ssh::connection::SshClient::connect(host) {
        Ok(client) => client,
        Err(e) => {
            return Ok(ModuleResult {
                stdout: String::new(),
                stderr: format!("Connection error: {}", e),
                changed: false,
                msg: format!("Failed to connect to host: {}", e),
            });
        }
    };
    
    execute(&ssh_client, args, false, "")
}

/// Execute the command module for ad-hoc commands on localhost
#[allow(dead_code)]
fn execute_adhoc_local(host: &Host, args: &Value) -> Result<ModuleResult> {
    if let Value::Mapping(map) = args {
        // Get the raw command to execute
        let cmd = match map.get(&Value::String("_raw_params".to_string())) {
            Some(Value::String(cmd)) => cmd,
            _ => return Err(anyhow!("Command module requires a command string")),
        };
        
        info!("Executing local command on host {}: {}", host.name, cmd);
        
        let (exit_code, stdout, stderr) = execute_local_command(cmd)?;
        
        let mut result = ModuleResult::default();
        result.stdout = stdout;
        result.stderr = stderr;
        
        if exit_code == 0 {
            result.changed = true;
            result.msg = format!("Command executed successfully (exit code: {})", exit_code);
        } else {
            result.msg = format!("Command failed with exit code: {}", exit_code);
        }
        
        Ok(result)
    } else {
        Err(anyhow!("Command module requires parameters"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execute_local_command_echo() {
        // This test should work on Unix and Windows
        let (code, out, err) = execute_local_command("echo hello").unwrap();
        assert_eq!(code, 0);
        assert_eq!(out.trim(), "hello");
        assert!(err.is_empty());
    }
} 