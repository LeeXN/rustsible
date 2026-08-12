use anyhow::Result;
use log::debug;
use serde_yaml::Value;

use crate::inventory::Host;
use crate::modules::{ModuleExecutor, ModuleResult};
use crate::ssh::connection::{shell_quote, SshConnection};

pub struct ShellModule;

impl ModuleExecutor for ShellModule {
    fn execute(
        connection: &dyn SshConnection,
        shell_args: &Value,
        use_become: bool,
        become_user: &str,
        check_mode: bool,
    ) -> Result<ModuleResult> {
        let shell_command = Self::extract_command_arg(shell_args)?;
        if shell_command.trim().is_empty() {
            return Err(anyhow::anyhow!("Shell module requires a non-empty command"));
        }
        if shell_command.contains('\0') {
            return Err(anyhow::anyhow!("Shell command cannot contain NUL bytes"));
        }
        if check_mode {
            return Ok(ModuleResult {
                stdout: String::new(),
                stderr: String::new(),
                rc: Some(0),
                changed: false,
                failed: false,
                msg: "Check mode: shell was not executed because it has no safe change prediction"
                    .to_string(),
            });
        }

        debug!(
            "Executing shell module payload ({} bytes)",
            shell_command.len()
        );

        // Wrap in shell to ensure proper environment and shell features
        let shell_wrapped = format!("sh -c {}", shell_quote(&shell_command));

        let (exit_code, stdout, stderr) =
            Self::execute_command(connection, &shell_wrapped, use_become, become_user)?;

        Self::process_command_result(
            exit_code,
            stdout,
            stderr,
            "Shell command executed successfully",
            "Shell command failed",
        )
    }
}

pub fn execute(
    connection: &dyn SshConnection,
    shell_args: &Value,
    use_become: bool,
    become_user: &str,
    check_mode: bool,
) -> Result<ModuleResult> {
    ShellModule::execute(connection, shell_args, use_become, become_user, check_mode)
}

pub fn execute_adhoc(
    host: &Host,
    shell_args: &Value,
    use_become: bool,
    become_user: &str,
    check_mode: bool,
) -> Result<ModuleResult> {
    ShellModule::execute_adhoc(host, shell_args, use_become, become_user, check_mode)
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
