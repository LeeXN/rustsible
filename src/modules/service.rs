use anyhow::{Result, Context};
use log::{info, warn, error};
use serde_yaml::Value;

use crate::inventory::Host;
use crate::ssh::connection::SshClient;
use crate::modules::ModuleResult;

/// Service states supported by the module
#[derive(Debug, Clone, PartialEq)]
enum ServiceState {
    Started,
    Stopped,
    Restarted,
    Reloaded,
}

impl ServiceState {
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "started" => Ok(ServiceState::Started),
            "stopped" => Ok(ServiceState::Stopped),
            "restarted" => Ok(ServiceState::Restarted),
            "reloaded" => Ok(ServiceState::Reloaded),
            _ => Err(anyhow::anyhow!("Invalid service state: {}", s)),
        }
    }
}

/// Execute service module with the given arguments
pub fn execute(ssh_client: &SshClient, args: &Value, use_become: bool, become_user: &str) -> Result<ModuleResult> {
    let map = match args {
        Value::Mapping(map) => map,
        _ => return Err(anyhow::anyhow!("Service module requires a mapping of arguments")),
    };

    // Get service name
    let name = match map.get(&Value::String("name".to_string())) {
        Some(Value::String(name)) => name,
        _ => return Err(anyhow::anyhow!("Service module requires a 'name' parameter")),
    };

    // Get desired state
    let state_str = match map.get(&Value::String("state".to_string())) {
        Some(Value::String(state)) => state,
        _ => return Err(anyhow::anyhow!("Service module requires a 'state' parameter")),
    };
    
    let state = ServiceState::from_str(state_str)
        .context("Failed to parse service state")?;

    // Check if we need to detect the init system
    let init_system = detect_init_system(ssh_client, use_become, become_user)?;
    info!("Detected init system: {}", init_system);

    // Build the command based on the detected init system
    let command = match (init_system.as_str(), &state) {
        ("systemd", ServiceState::Started) => format!("systemctl start {}", name),
        ("systemd", ServiceState::Stopped) => format!("systemctl stop {}", name),
        ("systemd", ServiceState::Restarted) => format!("systemctl restart {}", name),
        ("systemd", ServiceState::Reloaded) => format!("systemctl reload {}", name),
        
        ("sysvinit", ServiceState::Started) => format!("service {} start", name),
        ("sysvinit", ServiceState::Stopped) => format!("service {} stop", name),
        ("sysvinit", ServiceState::Restarted) => format!("service {} restart", name),
        ("sysvinit", ServiceState::Reloaded) => format!("service {} reload", name),
        
        ("upstart", ServiceState::Started) => format!("initctl start {}", name),
        ("upstart", ServiceState::Stopped) => format!("initctl stop {}", name),
        ("upstart", ServiceState::Restarted) => format!("initctl restart {}", name),
        ("upstart", ServiceState::Reloaded) => format!("initctl reload {}", name),
        
        _ => return Err(anyhow::anyhow!("Unsupported init system: {}", init_system)),
    };

    info!("Executing service command: {}", command);
    
    // Run the command with privilege escalation if needed
    let result = if use_become {
        ssh_client.execute_sudo_command(&command, become_user)?
    } else {
        ssh_client.execute_command(&command)?
    };
    
    let (exit_code, stdout, stderr) = result;
    
    if !stdout.trim().is_empty() {
        info!("Command stdout: {}", stdout);
    }
    
    if !stderr.trim().is_empty() {
        warn!("Command stderr: {}", stderr);
    }
    
    if exit_code != 0 {
        error!("Service command failed with exit code: {}", exit_code);
        return Err(anyhow::anyhow!("Service command failed with exit code: {}", exit_code));
    }
    
    info!("Service '{}' {}.", name, get_state_past_tense(&state));
    let state_str = match state {
        ServiceState::Started => "started",
        ServiceState::Stopped => "stopped",
        ServiceState::Restarted => "restarted",
        ServiceState::Reloaded => "reloaded",
    };
    Ok(ModuleResult {
        stdout: String::new(),
        stderr: String::new(),
        changed: true,
        msg: format!("Service {} state changed to {}", name, state_str),
    })
}

/// Execute service module in ad-hoc mode
pub fn execute_adhoc(host: &Host, service_args: &Value) -> Result<ModuleResult> {
    info!("Connecting to host: {}", host.name);
    let ssh_client = SshClient::connect(host)?;
    
    let service_name = get_param(service_args, "name")?;
    let state = get_param(service_args, "state").unwrap_or_else(|_| "started".to_string());
    
    execute(&ssh_client, service_args, false, "")?;
    
    Ok(ModuleResult {
        stdout: String::new(),
        stderr: String::new(),
        changed: true,
        msg: format!("Service {} state changed to {}", service_name, state),
    })
}

/// Detect the init system used by the remote host
fn detect_init_system(ssh_client: &SshClient, use_become: bool, become_user: &str) -> Result<String> {
    // Check for systemd
    let systemd_check = if use_become {
        ssh_client.execute_sudo_command("systemctl --version 2>/dev/null", become_user)
    } else {
        ssh_client.execute_command("systemctl --version 2>/dev/null")
    };
    
    if let Ok((exit_code, _, _)) = systemd_check {
        if exit_code == 0 {
            return Ok("systemd".to_string());
        }
    }
    
    // Check for upstart
    let upstart_check = if use_become {
        ssh_client.execute_sudo_command("initctl --version 2>/dev/null", become_user)
    } else {
        ssh_client.execute_command("initctl --version 2>/dev/null")
    };
    
    if let Ok((exit_code, _, _)) = upstart_check {
        if exit_code == 0 {
            return Ok("upstart".to_string());
        }
    }
    
    // Default to sysvinit
    Ok("sysvinit".to_string())
}

/// Get the past tense form of a service state for better log messages
fn get_state_past_tense(state: &ServiceState) -> &str {
    match state {
        ServiceState::Started => "started",
        ServiceState::Stopped => "stopped",
        ServiceState::Restarted => "restarted",
        ServiceState::Reloaded => "reloaded",
    }
}

// Helper to extract a string parameter from a YAML value
fn get_param(args: &Value, name: &str) -> Result<String> {
    match args {
        Value::Mapping(map) => {
            if let Some(Value::String(value)) = map.get(&Value::String(name.to_string())) {
                Ok(value.clone())
            } else {
                Err(anyhow::anyhow!("Missing required parameter: {}", name))
            }
        },
        _ => Err(anyhow::anyhow!("Arguments must be a mapping")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::{Value, Mapping};

    #[test]
    fn test_service_state_from_str() {
        assert_eq!(ServiceState::from_str("started").unwrap(), ServiceState::Started);
        assert_eq!(ServiceState::from_str("Stopped").unwrap(), ServiceState::Stopped);
        assert!(ServiceState::from_str("invalid").is_err());
    }

    #[test]
    fn test_get_state_past_tense() {
        assert_eq!(get_state_past_tense(&ServiceState::Restarted), "restarted");
        assert_eq!(get_state_past_tense(&ServiceState::Reloaded), "reloaded");
    }

    #[test]
    fn test_get_param() {
        let mut map = Mapping::new();
        map.insert(Value::String("key".to_string()), Value::String("value".to_string()));
        let args = Value::Mapping(map.clone());
        assert_eq!(get_param(&args, "key").unwrap(), "value");
        assert!(get_param(&args, "missing").is_err());
        let not_map = Value::String("string".to_string());
        assert!(get_param(&not_map, "key").is_err());
    }
} 