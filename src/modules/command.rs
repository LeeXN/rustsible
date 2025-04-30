use anyhow::Result;
use log::{info, warn, error};
use serde_yaml::Value;

use crate::inventory::Host;
use crate::ssh::connection::SshClient;
use crate::modules::ModuleResult;

pub fn execute(ssh_client: &SshClient, command_args: &Value, use_become: bool, become_user: &str) -> Result<ModuleResult> {
    let command_str = match command_args {
        Value::String(cmd) => cmd.clone(),
        Value::Mapping(map) => {
            if let Some(Value::String(cmd)) = map.get(&Value::String("cmd".to_string())) {
                cmd.clone()
            } else {
                return Err(anyhow::anyhow!("Command module requires a valid command string"));
            }
        },
        _ => return Err(anyhow::anyhow!("Command module requires a valid command string")),
    };
    
    info!("Executing command: {}", command_str);
    
    let result = if use_become {
        ssh_client.execute_sudo_command(&command_str, become_user)?
    } else {
        ssh_client.execute_command(&command_str)?
    };
    
    let (exit_code, stdout, stderr) = result;
    
    // 构建结果输出
    let module_result = ModuleResult {
        stdout,
        stderr,
        changed: true,  // 通常命令被认为是修改了系统状态
        msg: format!("Command executed with exit code: {}", exit_code),
    };
    
    if !module_result.stdout.trim().is_empty() {
        info!("Command stdout: {}", module_result.stdout);
    }
    
    if !module_result.stderr.trim().is_empty() {
        warn!("Command stderr: {}", module_result.stderr);
    }
    
    if exit_code != 0 {
        error!("Command failed with exit code: {}", exit_code);
        return Err(anyhow::anyhow!("Command failed with exit code: {}", exit_code));
    }
    
    info!("Command executed successfully");
    Ok(module_result)
}

pub fn execute_adhoc(host: &Host, command_args: &Value) -> Result<ModuleResult> {
    info!("Connecting to host: {}", host.name);
    let ssh_client = SshClient::connect(host)?;
    
    execute(&ssh_client, command_args, false, "")
} 