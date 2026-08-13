use anyhow::{bail, Context, Result};
use serde_yaml::Mapping;

use crate::modules::file::quote_posix_shell_arg;
use crate::ssh::connection::SshConnection;

/// Run a small module payload on the managed host. Python is the same remote
/// runtime required by ansible-core, so downloads and fact discovery observe
/// the managed host's DNS, proxy, and TLS configuration.
pub(crate) fn execute_json_mapping(
    connection: &dyn SshConnection,
    script: &str,
    arguments: &[String],
    use_become: bool,
    become_user: &str,
) -> Result<Mapping> {
    let quoted = arguments
        .iter()
        .map(|argument| quote_posix_shell_arg(argument))
        .collect::<Result<Vec<_>>>()?;
    let command = if quoted.is_empty() {
        "python3 -".to_string()
    } else {
        format!("python3 - {}", quoted.join(" "))
    };
    let (code, stdout, stderr) = if use_become {
        connection.execute_sudo_command_with_input(&command, become_user, script.as_bytes())?
    } else {
        connection.execute_command_with_input(&command, script.as_bytes())?
    };
    if code == 127 {
        bail!("Python 3 is required on the managed host");
    }
    if code != 0 {
        bail!(
            "Remote Python module failed (exit {}): {}",
            code,
            stderr.trim()
        );
    }

    let value: serde_json::Value =
        serde_json::from_str(&stdout).context("Remote Python module returned invalid JSON")?;
    let value = serde_yaml::to_value(value).context("Failed to decode remote module result")?;
    value
        .as_mapping()
        .cloned()
        .context("Remote Python module result must be an object")
}
