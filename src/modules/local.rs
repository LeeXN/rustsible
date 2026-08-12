use anyhow::Result;

use crate::inventory::Host;
use crate::modules::ModuleResult;
use crate::ssh::connection::{Connection, LocalConnection, SshConnection};

/// Backward-compatible helper. New execution paths select a `Connection` and
/// invoke the same module implementation for local and SSH targets.
pub fn execute_local_command(command: &str) -> Result<(i32, String, String)> {
    let host = Host::new("localhost");
    let connection = LocalConnection::new(&host)?;
    connection.execute_command(command)
}

pub fn execute(
    connection: &dyn SshConnection,
    args: &serde_yaml::Value,
    use_become: bool,
    become_user: &str,
    check_mode: bool,
) -> Result<ModuleResult> {
    crate::modules::command::execute(connection, args, use_become, become_user, check_mode)
}

pub fn execute_adhoc(
    host: &Host,
    args: &serde_yaml::Value,
    use_become: bool,
    become_user: &str,
    check_mode: bool,
) -> Result<ModuleResult> {
    let connection = Connection::connect(host)?;
    execute(
        connection.as_connection(),
        args,
        use_become,
        become_user,
        check_mode,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execute_local_command_echo() {
        let (code, out, err) = execute_local_command("echo hello").unwrap();
        assert_eq!(code, 0);
        assert_eq!(out.trim(), "hello");
        assert!(err.is_empty());
    }
}
