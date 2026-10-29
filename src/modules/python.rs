use anyhow::{Context, Result};
use serde_yaml::Mapping;

use crate::modules::file::quote_posix_shell_arg;
use crate::ssh::connection::SshConnection;

/// Run a small module payload on the managed host. Python is the same remote
/// runtime still used by the transitional downloader, which observes the
/// managed host's DNS, proxy, and TLS configuration.
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
    if code != 0 {
        let error = if code == 127 {
            anyhow::anyhow!("Python 3 is required on the managed host")
        } else {
            anyhow::anyhow!(
                "Remote Python module failed (exit {code}): {}",
                stderr.trim()
            )
        };
        return Err(crate::modules::result::command_error(
            error, code, stdout, stderr,
        ));
    }

    let decode = (|| {
        let value: serde_json::Value =
            serde_json::from_str(&stdout).context("Remote Python module returned invalid JSON")?;
        let value = serde_yaml::to_value(value).context("Failed to decode remote module result")?;
        value
            .as_mapping()
            .cloned()
            .context("Remote Python module result must be an object")
    })();
    decode.map_err(|error| {
        crate::ssh::connection::result_unavailable(crate::modules::result::command_error(
            error, code, stdout, stderr,
        ))
    })
}
