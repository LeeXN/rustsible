use anyhow::Result;
use log::{info, warn};
use serde_yaml::Value;

use crate::inventory::Host;
use crate::ssh::connection::SshClient;
use crate::modules::{ModuleResult, ModuleExecutor};

pub struct CommandModule;

impl ModuleExecutor for CommandModule {
    fn execute(ssh_client: &SshClient, command_args: &Value, use_become: bool, become_user: &str) -> Result<ModuleResult> {
        let command_str = Self::extract_command_arg(command_args)?;
        
        info!("Executing command: {}", command_str);
        
        let (exit_code, stdout, stderr) = Self::execute_command(ssh_client, &command_str, use_become, become_user)?;
        
        // 处理结果
        if !stdout.trim().is_empty() {
            info!("Command stdout: {}", stdout);
        }
        
        if !stderr.trim().is_empty() {
            warn!("Command stderr: {}", stderr);
        }
        
        Self::process_command_result(
            exit_code, 
            stdout, 
            stderr, 
            "Command executed successfully", 
            "Command failed"
        )
    }
}

pub fn execute(ssh_client: &SshClient, command_args: &Value, use_become: bool, become_user: &str) -> Result<ModuleResult> {
    CommandModule::execute(ssh_client, command_args, use_become, become_user)
}

pub fn execute_adhoc(host: &Host, command_args: &Value) -> Result<ModuleResult> {
    CommandModule::execute_adhoc(host, command_args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::{Value, Mapping};

    #[test]
    fn test_extract_command_arg() {
        // 测试字符串参数
        let string_arg = Value::String("echo 'hello world'".to_string());
        assert_eq!(
            CommandModule::extract_command_arg(&string_arg).unwrap(), 
            "echo 'hello world'"
        );

        // 测试映射参数
        let mut map = Mapping::new();
        map.insert(
            Value::String("cmd".to_string()),
            Value::String("echo 'hello world'".to_string())
        );
        let mapped_arg = Value::Mapping(map);
        assert_eq!(
            CommandModule::extract_command_arg(&mapped_arg).unwrap(), 
            "echo 'hello world'"
        );

        // 测试错误情况：无效参数类型
        let invalid_arg = Value::Sequence(vec![]);
        assert!(CommandModule::extract_command_arg(&invalid_arg).is_err());
    }
} 