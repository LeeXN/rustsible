use anyhow::Result;
use log::debug;
use serde_yaml::Value;

use crate::inventory::Host;
use crate::modules::file::quote_posix_shell_arg;
use crate::modules::tokenize_args;
use crate::modules::{ModuleExecutor, ModuleResult};
use crate::ssh::connection::SshConnection;

pub struct CommandModule;

impl ModuleExecutor for CommandModule {
    fn execute(
        connection: &dyn SshConnection,
        command_args: &Value,
        use_become: bool,
        become_user: &str,
        check_mode: bool,
    ) -> Result<ModuleResult> {
        let command_str = Self::extract_command_arg(command_args)?;
        let command = tokenize_args(&command_str)?;
        if command.is_empty() {
            return Err(anyhow::anyhow!(
                "Command module requires a non-empty command"
            ));
        }
        // SSH transports commands through a remote shell. Quote every parsed
        // argv element so `command` keeps argv semantics; callers that need
        // redirects, pipes or expansion must explicitly use `shell`.
        let command_str = command
            .iter()
            .map(|argument| quote_posix_shell_arg(argument))
            .collect::<Result<Vec<_>>>()?
            .join(" ");

        if check_mode {
            return Ok(ModuleResult {
                stdout: String::new(),
                stderr: String::new(),
                rc: Some(0),
                changed: false,
                failed: false,
                msg:
                    "Check mode: command was not executed because it has no safe change prediction"
                        .to_string(),
                values: Default::default(),
            });
        }

        debug!(
            "Executing command module payload ({} bytes)",
            command_str.len()
        );

        let (exit_code, stdout, stderr) =
            Self::execute_command(connection, &command_str, use_become, become_user)?;

        Self::process_command_result(
            exit_code,
            stdout,
            stderr,
            "Command executed successfully",
            "Command failed",
        )
    }
}

pub fn execute(
    connection: &dyn SshConnection,
    command_args: &Value,
    use_become: bool,
    become_user: &str,
    check_mode: bool,
) -> Result<ModuleResult> {
    CommandModule::execute(
        connection,
        command_args,
        use_become,
        become_user,
        check_mode,
    )
}

pub fn execute_adhoc(
    host: &Host,
    command_args: &Value,
    use_become: bool,
    become_user: &str,
    check_mode: bool,
) -> Result<ModuleResult> {
    CommandModule::execute_adhoc(host, command_args, use_become, become_user, check_mode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::connection::MockSshConnection;
    use serde_yaml::{Mapping, Value};

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
            Value::String("echo 'hello world'".to_string()),
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

    #[test]
    fn command_uses_argv_semantics_instead_of_shell_syntax() {
        let command = tokenize_args("printf '%s' 'hello; touch /tmp/not-created'").unwrap();
        let quoted = command
            .iter()
            .map(|argument| quote_posix_shell_arg(argument))
            .collect::<Result<Vec<_>>>()
            .unwrap()
            .join(" ");

        assert_eq!(quoted, "'printf' '%s' 'hello; touch /tmp/not-created'");
        assert!(tokenize_args("'unterminated").is_err());
    }

    #[test]
    fn command_executes_reports_failures_and_honors_check_mode() {
        let mut success = MockSshConnection::new();
        success
            .expect_execute_command()
            .withf(|command| command == "'printf' '%s' 'hello world'")
            .once()
            .returning(|_| Ok((0, "hello world".to_string(), String::new())));
        let result = execute(
            &success,
            &Value::String("printf '%s' 'hello world'".to_string()),
            false,
            "root",
            false,
        )
        .unwrap();
        assert_eq!(result.stdout, "hello world");
        assert!(result.changed);

        let mut failure = MockSshConnection::new();
        failure
            .expect_execute_sudo_command()
            .withf(|command, user| command == "'false'" && user == "operator")
            .once()
            .returning(|_, _| Ok((2, String::new(), "denied".to_string())));
        let failed = execute(
            &failure,
            &Value::String("false".to_string()),
            true,
            "operator",
            false,
        )
        .unwrap();
        assert!(failed.failed);
        assert!(failed.msg.contains("denied"));

        let check = execute(
            &MockSshConnection::new(),
            &Value::String("true".to_string()),
            false,
            "root",
            true,
        )
        .unwrap();
        assert!(!check.changed);
        assert!(execute(
            &MockSshConnection::new(),
            &Value::String("  ".to_string()),
            false,
            "root",
            false
        )
        .is_err());
    }
}
