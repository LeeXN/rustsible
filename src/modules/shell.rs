use anyhow::Result;
use log::{info, warn, error};
use serde_yaml::Value;

use crate::inventory::Host;
use crate::ssh::connection::SshClient;
use crate::modules::ModuleResult;

pub fn execute(ssh_client: &SshClient, shell_args: &Value, use_become: bool, become_user: &str) -> Result<ModuleResult> {
    let shell_command = match shell_args {
        Value::String(cmd) => cmd.clone(),
        Value::Mapping(map) => {
            if let Some(Value::String(cmd)) = map.get(&Value::String("cmd".to_string())) {
                cmd.clone()
            } else {
                return Err(anyhow::anyhow!("Shell module requires a valid shell command"));
            }
        },
        _ => return Err(anyhow::anyhow!("Shell module requires a valid shell command")),
    };
    
    info!("Executing shell command: {}", shell_command);
    
    // Wrap in shell to ensure proper environment and shell features
    let shell_wrapped = format!("sh -c '{}'", shell_command.replace("'", "'\\''"));
    
    let result = if use_become {
        ssh_client.execute_sudo_command(&shell_wrapped, become_user)?
    } else {
        ssh_client.execute_command(&shell_wrapped)?
    };
    
    let (exit_code, stdout, stderr) = result;
    
    // 构建结果输出
    let module_result = ModuleResult {
        stdout,
        stderr: stderr.clone(),
        changed: true,  // 通常shell命令被认为是修改了系统状态
        msg: format!("Shell command executed with exit code: {}", exit_code),
    };
    
    if !module_result.stdout.trim().is_empty() {
        info!("Shell stdout: {}", module_result.stdout);
    }
    
    if !module_result.stderr.trim().is_empty() {
        warn!("Shell stderr: {}", module_result.stderr);
    }
    
    if exit_code != 0 {
        error!("Shell command failed with exit code: {}", exit_code);
        let error_msg = if stderr.trim().is_empty() {
            format!("Shell command failed with exit code: {}", exit_code)
        } else {
            format!("Shell command failed with exit code: {}\nError: {}", exit_code, stderr.trim())
        };
        return Err(anyhow::anyhow!(error_msg));
    }
    
    info!("Shell command executed successfully");
    Ok(module_result)
}

pub fn execute_adhoc(host: &Host, shell_args: &Value) -> Result<ModuleResult> {
    info!("Connecting to host: {}", host.name);
    let ssh_client = SshClient::connect(host)?;
    
    execute(&ssh_client, shell_args, false, "")
}

#[cfg(test)]
mod tests {
    use serde_yaml::Value;

    #[test]
    fn test_shell_command_parsing() {
        // 测试字符串参数
        let _string_arg = Value::String("echo 'hello world'".to_string());
        let _mapped_arg = Value::Mapping({
            let mut map = serde_yaml::Mapping::new();
            map.insert(
                Value::String("cmd".to_string()),
                Value::String("echo 'hello world'".to_string())
            );
            map
        });

        
        // 测试错误情况：无效参数类型
        let invalid_arg = Value::Sequence(vec![]);
        let result = match invalid_arg {
            Value::String(cmd) => Ok(cmd.clone()),
            Value::Mapping(map) => {
                if let Some(Value::String(cmd)) = map.get(&Value::String("cmd".to_string())) {
                    Ok(cmd.clone())
                } else {
                    Err(anyhow::anyhow!("Shell module requires a valid shell command"))
                }
            },
            _ => Err(anyhow::anyhow!("Shell module requires a valid shell command")),
        };
        assert!(result.is_err());
    }
} 