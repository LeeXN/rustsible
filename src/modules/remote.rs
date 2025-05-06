use anyhow::Result;
use log::{info, error};
use crate::ssh::connection::SshClient;

/// Set remote file mode (permissions) using chmod.
/// Returns an error if the command fails.
pub fn set_file_mode(ssh_client: &SshClient, path: &str, mode: &str, use_become: bool, become_user: &str) -> Result<()> {
    info!("Setting file mode: {} -> {}", path, mode);
    let cmd = format!("chmod {} {}", mode, path);
    let (exit_code, _, stderr) = if use_become {
        ssh_client.execute_sudo_command(&cmd, become_user)?
    } else {
        ssh_client.execute_command(&cmd)?
    };
    if exit_code != 0 {
        error!("Failed to set file mode: {}", stderr);
        return Err(anyhow::anyhow!("Failed to set file mode: {}", stderr));
    }
    Ok(())
}

/// Set remote file ownership (user/group) using chown.
/// Returns an error if the command fails.
pub fn set_ownership(ssh_client: &SshClient, path: &str, owner: Option<&str>, group: Option<&str>, use_become: bool, become_user: &str) -> Result<()> {
    let ownership = match (owner, group) {
        (Some(o), Some(g)) => format!("{}:{}", o, g),
        (Some(o), None) => o.to_string(),
        (None, Some(g)) => format!(":{}", g),
        (None, None) => return Ok(()),
    };
    info!("Setting ownership: {} -> {}", path, ownership);
    let cmd = format!("chown {} {}", ownership, path);
    let (exit_code, _, stderr) = if use_become {
        ssh_client.execute_sudo_command(&cmd, become_user)?
    } else {
        ssh_client.execute_command(&cmd)?
    };
    if exit_code != 0 {
        error!("Failed to set ownership: {}", stderr);
        return Err(anyhow::anyhow!("Failed to set ownership: {}", stderr));
    }
    Ok(())
} 