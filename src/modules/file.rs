use anyhow::Result;
use log::{info, error};
use serde_yaml::Value;

use crate::inventory::Host;
use crate::ssh::connection::SshClient;
use crate::modules::ModuleResult;

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

pub fn execute(ssh_client: &SshClient, file_args: &Value, use_become: bool, become_user: &str) -> Result<()> {
    // Extract path parameter (required)
    let path = match file_args {
        Value::Mapping(map) => {
            if let Some(Value::String(path)) = map.get(&Value::String("path".to_string())) {
                path.clone()
            } else if let Some(Value::String(path)) = map.get(&Value::String("dest".to_string())) {
                path.clone()
            } else {
                return Err(anyhow::anyhow!("File module requires a path parameter"));
            }
        },
        _ => return Err(anyhow::anyhow!("File module requires a mapping of parameters")),
    };
    
    // Extract state parameter
    let state = if let Value::Mapping(map) = file_args {
        if let Some(Value::String(state_str)) = map.get(&Value::String("state".to_string())) {
            FileState::from_str(state_str)?
        } else {
            FileState::File // Default state
        }
    } else {
        FileState::File
    };
    
    // Extract mode parameter
    let mode = if let Value::Mapping(map) = file_args {
        if let Some(Value::String(mode_str)) = map.get(&Value::String("mode".to_string())) {
            Some(mode_str.clone())
        } else {
            None
        }
    } else {
        None
    };
    
    // Extract owner parameter
    let owner = if let Value::Mapping(map) = file_args {
        if let Some(Value::String(owner_str)) = map.get(&Value::String("owner".to_string())) {
            Some(owner_str.clone())
        } else {
            None
        }
    } else {
        None
    };
    
    // Extract group parameter
    let group = if let Value::Mapping(map) = file_args {
        if let Some(Value::String(group_str)) = map.get(&Value::String("group".to_string())) {
            Some(group_str.clone())
        } else {
            None
        }
    } else {
        None
    };
    
    // Handle different file states
    match state {
        FileState::File => {
            create_file(ssh_client, &path, use_become, become_user)?;
            if let Some(mode_str) = &mode {
                set_file_mode(ssh_client, &path, mode_str, use_become, become_user)?;
            }
        },
        FileState::Directory => {
            create_directory(ssh_client, &path, use_become, become_user)?;
            if let Some(mode_str) = &mode {
                set_file_mode(ssh_client, &path, mode_str, use_become, become_user)?;
            }
        },
        FileState::Absent => {
            remove_file(ssh_client, &path, use_become, become_user)?;
        },
        FileState::Touch => {
            touch_file(ssh_client, &path, use_become, become_user)?;
            if let Some(mode_str) = &mode {
                set_file_mode(ssh_client, &path, mode_str, use_become, become_user)?;
            }
        },
        FileState::Link => {
            // Link requires a src parameter
            let src = if let Value::Mapping(map) = file_args {
                if let Some(Value::String(src_str)) = map.get(&Value::String("src".to_string())) {
                    src_str.clone()
                } else {
                    return Err(anyhow::anyhow!("Link state requires a src parameter"));
                }
            } else {
                return Err(anyhow::anyhow!("Link state requires a src parameter"));
            };
            
            create_symlink(ssh_client, &src, &path, use_become, become_user)?;
        },
    }
    
    // Set owner/group if specified
    if owner.is_some() || group.is_some() {
        set_ownership(ssh_client, &path, owner.as_deref(), group.as_deref(), use_become, become_user)?;
    }
    
    info!("File operation completed successfully");
    Ok(())
}

fn create_file(ssh_client: &SshClient, path: &str, use_become: bool, become_user: &str) -> Result<()> {
    info!("Creating file: {}", path);
    
    let cmd = format!("[ -f {} ] || touch {}", path, path);
    let (exit_code, _, stderr) = if use_become {
        ssh_client.execute_sudo_command(&cmd, become_user)?
    } else {
        ssh_client.execute_command(&cmd)?
    };
    
    if exit_code != 0 {
        error!("Failed to create file: {}", stderr);
        return Err(anyhow::anyhow!("Failed to create file: {}", stderr));
    }
    
    Ok(())
}

fn create_directory(ssh_client: &SshClient, path: &str, use_become: bool, become_user: &str) -> Result<()> {
    info!("Creating directory: {}", path);
    
    let cmd = format!("mkdir -p {}", path);
    let (exit_code, _, stderr) = if use_become {
        ssh_client.execute_sudo_command(&cmd, become_user)?
    } else {
        ssh_client.execute_command(&cmd)?
    };
    
    if exit_code != 0 {
        error!("Failed to create directory: {}", stderr);
        return Err(anyhow::anyhow!("Failed to create directory: {}", stderr));
    }
    
    Ok(())
}

fn remove_file(ssh_client: &SshClient, path: &str, use_become: bool, become_user: &str) -> Result<()> {
    info!("Removing file/directory: {}", path);
    
    let cmd = format!("rm -rf {}", path);
    let (exit_code, _, stderr) = if use_become {
        ssh_client.execute_sudo_command(&cmd, become_user)?
    } else {
        ssh_client.execute_command(&cmd)?
    };
    
    if exit_code != 0 {
        error!("Failed to remove file/directory: {}", stderr);
        return Err(anyhow::anyhow!("Failed to remove file/directory: {}", stderr));
    }
    
    Ok(())
}

fn touch_file(ssh_client: &SshClient, path: &str, use_become: bool, become_user: &str) -> Result<()> {
    info!("Touching file: {}", path);
    
    let cmd = format!("touch {}", path);
    let (exit_code, _, stderr) = if use_become {
        ssh_client.execute_sudo_command(&cmd, become_user)?
    } else {
        ssh_client.execute_command(&cmd)?
    };
    
    if exit_code != 0 {
        error!("Failed to touch file: {}", stderr);
        return Err(anyhow::anyhow!("Failed to touch file: {}", stderr));
    }
    
    Ok(())
}

fn create_symlink(ssh_client: &SshClient, src: &str, dest: &str, use_become: bool, become_user: &str) -> Result<()> {
    info!("Creating symlink: {} -> {}", dest, src);
    
    let cmd = format!("ln -sf {} {}", src, dest);
    let (exit_code, _, stderr) = if use_become {
        ssh_client.execute_sudo_command(&cmd, become_user)?
    } else {
        ssh_client.execute_command(&cmd)?
    };
    
    if exit_code != 0 {
        error!("Failed to create symlink: {}", stderr);
        return Err(anyhow::anyhow!("Failed to create symlink: {}", stderr));
    }
    
    Ok(())
}

fn set_file_mode(ssh_client: &SshClient, path: &str, mode: &str, use_become: bool, become_user: &str) -> Result<()> {
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

fn set_ownership(ssh_client: &SshClient, path: &str, owner: Option<&str>, group: Option<&str>, use_become: bool, become_user: &str) -> Result<()> {
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

pub fn execute_adhoc(host: &Host, file_args: &Value) -> Result<ModuleResult> {
    info!("Connecting to host: {}", host.name);
    let ssh_client = SshClient::connect(host)?;
    
    // 解析文件路径
    let path = get_param(file_args, "path")?;
    let state = get_param(file_args, "state").unwrap_or_else(|_| "file".to_string());
    
    execute(&ssh_client, file_args, false, "")?;
    
    Ok(ModuleResult {
        stdout: String::new(),
        stderr: String::new(),
        changed: true,
        msg: format!("File {} state changed to {}", path, state),
    })
}

// Helper to extract a string parameter from a YAML value
fn get_param(args: &Value, name: &str) -> Result<String> {
    match args {
        Value::Mapping(map) => {
            if let Some(Value::String(value)) = map.get(&Value::String(name.to_string())) {
                Ok(value.clone())
            } else {
                Err(anyhow::anyhow!("Missing required parameter: {}", name))
            }
        },
        _ => Err(anyhow::anyhow!("Arguments must be a mapping")),
    }
} 