use anyhow::Result;
use log::info;
use serde_yaml::Value;

use crate::inventory::Host;
use crate::ssh::connection::SshClient;
use crate::modules::ModuleResult;
use crate::modules::param::{get_param, get_optional_param};
use crate::modules::remote::{set_file_mode, set_ownership};

#[derive(Debug, PartialEq)]
enum FileState {
    File,
    Directory,
    Link,
    Absent,
    Touch,
}

impl FileState {
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "file" => Ok(FileState::File),
            "directory" | "dir" => Ok(FileState::Directory),
            "link" => Ok(FileState::Link),
            "absent" => Ok(FileState::Absent),
            "touch" => Ok(FileState::Touch),
            _ => Err(anyhow::anyhow!("Invalid file state: {}", s)),
        }
    }
}

/// Execute the file module logic: create/remove/touch/set permissions/ownership for files or directories.
pub fn execute(ssh_client: &SshClient, file_args: &Value, use_become: bool, become_user: &str) -> Result<()> {
    let path = get_param::<String>(file_args, "path").or_else(|_| get_param::<String>(file_args, "dest"))?;
    let state = if let Value::Mapping(map) = file_args {
        if let Some(Value::String(state_str)) = map.get(&Value::String("state".to_string())) {
            FileState::from_str(state_str)?
        } else {
            FileState::File
        }
    } else {
        FileState::File
    };
    let mode = get_optional_param::<String>(file_args, "mode");
    let owner = get_optional_param::<String>(file_args, "owner");
    let group = get_optional_param::<String>(file_args, "group");
    match state {
        FileState::File => {
            create_file(ssh_client, &path, use_become, become_user)?;
            if let Some(mode_str) = mode.as_deref() {
                set_file_mode(ssh_client, &path, mode_str, use_become, become_user)?;
            }
        },
        FileState::Directory => {
            create_directory(ssh_client, &path, use_become, become_user)?;
            if let Some(mode_str) = mode.as_deref() {
                set_file_mode(ssh_client, &path, mode_str, use_become, become_user)?;
            }
        },
        FileState::Absent => {
            remove_file(ssh_client, &path, use_become, become_user)?;
        },
        FileState::Touch => {
            touch_file(ssh_client, &path, use_become, become_user)?;
            if let Some(mode_str) = mode.as_deref() {
                set_file_mode(ssh_client, &path, mode_str, use_become, become_user)?;
            }
        },
        FileState::Link => {
            let src = get_param::<String>(file_args, "src")?;
            create_symlink(ssh_client, &src, &path, use_become, become_user)?;
        },
    }
    if owner.is_some() || group.is_some() {
        set_ownership(ssh_client, &path, owner.as_deref(), group.as_deref(), use_become, become_user)?;
    }
    info!("File operation completed successfully");
    Ok(())
}

/// Create a file if it does not exist.
fn create_file(ssh_client: &SshClient, path: &str, use_become: bool, become_user: &str) -> Result<()> {
    info!("Creating file: {}", path);
    let cmd = format!("[ -f {} ] || touch {}", path, path);
    let (exit_code, _, stderr) = if use_become {
        ssh_client.execute_sudo_command(&cmd, become_user)?
    } else {
        ssh_client.execute_command(&cmd)?
    };
    if exit_code != 0 {
        log::error!("Failed to create file: {}", stderr);
        return Err(anyhow::anyhow!("Failed to create file: {}", stderr));
    }
    Ok(())
}

/// Create a directory if it does not exist.
fn create_directory(ssh_client: &SshClient, path: &str, use_become: bool, become_user: &str) -> Result<()> {
    info!("Creating directory: {}", path);
    let cmd = format!("mkdir -p {}", path);
    let (exit_code, _, stderr) = if use_become {
        ssh_client.execute_sudo_command(&cmd, become_user)?
    } else {
        ssh_client.execute_command(&cmd)?
    };
    if exit_code != 0 {
        log::error!("Failed to create directory: {}", stderr);
        return Err(anyhow::anyhow!("Failed to create directory: {}", stderr));
    }
    Ok(())
}

/// Remove a file or directory.
fn remove_file(ssh_client: &SshClient, path: &str, use_become: bool, become_user: &str) -> Result<()> {
    info!("Removing file/directory: {}", path);
    let cmd = format!("rm -rf {}", path);
    let (exit_code, _, stderr) = if use_become {
        ssh_client.execute_sudo_command(&cmd, become_user)?
    } else {
        ssh_client.execute_command(&cmd)?
    };
    if exit_code != 0 {
        log::error!("Failed to remove file/directory: {}", stderr);
        return Err(anyhow::anyhow!("Failed to remove file/directory: {}", stderr));
    }
    Ok(())
}

/// Touch a file (update timestamp or create if not exists).
fn touch_file(ssh_client: &SshClient, path: &str, use_become: bool, become_user: &str) -> Result<()> {
    info!("Touching file: {}", path);
    let cmd = format!("touch {}", path);
    let (exit_code, _, stderr) = if use_become {
        ssh_client.execute_sudo_command(&cmd, become_user)?
    } else {
        ssh_client.execute_command(&cmd)?
    };
    if exit_code != 0 {
        log::error!("Failed to touch file: {}", stderr);
        return Err(anyhow::anyhow!("Failed to touch file: {}", stderr));
    }
    Ok(())
}

/// Create a symbolic link.
fn create_symlink(ssh_client: &SshClient, src: &str, dest: &str, use_become: bool, become_user: &str) -> Result<()> {
    info!("Creating symlink: {} -> {}", dest, src);
    let cmd = format!("ln -sf {} {}", src, dest);
    let (exit_code, _, stderr) = if use_become {
        ssh_client.execute_sudo_command(&cmd, become_user)?
    } else {
        ssh_client.execute_command(&cmd)?
    };
    if exit_code != 0 {
        log::error!("Failed to create symlink: {}", stderr);
        return Err(anyhow::anyhow!("Failed to create symlink: {}", stderr));
    }
    Ok(())
}

/// Execute the file module in ad-hoc mode for a single host.
pub fn execute_adhoc(host: &Host, file_args: &Value) -> Result<ModuleResult> {
    info!("Connecting to host: {}", host.name);
    let ssh_client = SshClient::connect(host)?;
    execute(&ssh_client, file_args, false, "")?;
    Ok(ModuleResult {
        stdout: String::new(),
        stderr: String::new(),
        changed: true,
        msg: format!("File {} state changed to {}", get_param::<String>(file_args, "path")?, get_param::<String>(file_args, "state")?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::{Value, Mapping};

    #[test]
    fn test_file_param_extract_ok() {
        let mut map = Mapping::new();
        map.insert(Value::String("path".to_string()), Value::String("/tmp/f".to_string()));
        let args = Value::Mapping(map);
        assert_eq!(crate::modules::param::get_param::<String>(&args, "path").unwrap(), "/tmp/f");
    }

    #[test]
    fn test_file_param_missing() {
        let map = Mapping::new();
        let args = Value::Mapping(map);
        assert!(crate::modules::param::get_param::<String>(&args, "path").is_err());
    }
} 