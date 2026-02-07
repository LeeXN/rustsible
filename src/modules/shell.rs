use anyhow::Result;
use log::{info, warn};
use serde_yaml::Value;

use crate::inventory::Host;
use crate::modules::{ModuleExecutor, ModuleResult};
use crate::ssh::connection::SshClient;

pub struct ShellModule;

impl ModuleExecutor for ShellModule {
    fn execute(
        ssh_client: &SshClient,
        shell_args: &Value,
        use_become: bool,
        become_user: &str,
    ) -> Result<ModuleResult> {
        let shell_command = Self::extract_command_arg(shell_args)?;

        info!("Executing shell command: {}", shell_command);

        // Wrap in shell to ensure proper environment and shell features
        let shell_wrapped = format!("sh -c '{}'", shell_command.replace("'", "'\\''"));

        let (exit_code, stdout, stderr) =
            Self::execute_command(ssh_client, &shell_wrapped, use_become, become_user)?;

        // 处理结果
        if !stdout.trim().is_empty() {
            info!("Shell stdout: {}", stdout);
        }

        if !stderr.trim().is_empty() {
            warn!("Shell stderr: {}", stderr);
        }

        Self::process_command_result(
            exit_code,
            stdout,
            stderr,
            "Shell command executed successfully",
            "Shell command failed",
        )
    }
}

pub fn execute_adhoc(host: &Host, shell_args: &Value) -> Result<ModuleResult> {
    ShellModule::execute_adhoc(host, shell_args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::{Mapping, Value};

    #[test]
    fn test_extract_command_arg() {
        // 测试字符串参数
        let string_arg = Value::String("echo 'hello world'".to_string());
        assert_eq!(
            ShellModule::extract_command_arg(&string_arg).unwrap(),
            "echo 'hello world'"
        );

        // 测试映射参数
        let mut map = Mapping::new();
        map.insert(
            Value::String("cmd".to_string()),
            Value::String("echo 'hello world'".to_string()),
        );
        let mapped_arg = Value::Mapping(map);
        assert_eq!(
            ShellModule::extract_command_arg(&mapped_arg).unwrap(),
            "echo 'hello world'"
        );

        // 测试错误情况：无效参数类型
        let invalid_arg = Value::Sequence(vec![]);
        assert!(ShellModule::extract_command_arg(&invalid_arg).is_err());
    }
}
