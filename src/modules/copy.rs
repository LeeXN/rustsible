use anyhow::{Result, Context};
use log::{info, error};
use serde_yaml::Value;
use std::path::Path;

use crate::inventory::Host;
use crate::ssh::connection::SshClient;
use crate::modules::ModuleResult;

pub fn execute(ssh_client: &SshClient, copy_args: &Value, use_become: bool, become_user: &str) -> Result<()> {
    // Extract src parameter (required)
    let src = match copy_args {
        Value::Mapping(map) => {
            if let Some(Value::String(src)) = map.get(&Value::String("src".to_string())) {
                src.clone()
            } else {
                return Err(anyhow::anyhow!("Copy module requires a src parameter"));
            }
        },
        _ => return Err(anyhow::anyhow!("Copy module requires a mapping of parameters")),
    };
    
    // Extract dest parameter (required)
    let dest = match copy_args {
        Value::Mapping(map) => {
            if let Some(Value::String(dest)) = map.get(&Value::String("dest".to_string())) {
                dest.clone()
            } else {
                return Err(anyhow::anyhow!("Copy module requires a dest parameter"));
            }
        },
        _ => return Err(anyhow::anyhow!("Copy module requires a mapping of parameters")),
    };
    
    // Extract mode parameter (optional)
    let mode = if let Value::Mapping(map) = copy_args {
        if let Some(Value::String(mode_str)) = map.get(&Value::String("mode".to_string())) {
            Some(mode_str.clone())
        } else {
            None
        }
    } else {
        None
    };
    
    // Extract owner parameter (optional)
    let owner = if let Value::Mapping(map) = copy_args {
        if let Some(Value::String(owner_str)) = map.get(&Value::String("owner".to_string())) {
            Some(owner_str.clone())
        } else {
            None
        }
    } else {
        None
    };
    
    // Extract group parameter (optional)
    let group = if let Value::Mapping(map) = copy_args {
        if let Some(Value::String(group_str)) = map.get(&Value::String("group".to_string())) {
            Some(group_str.clone())
        } else {
            None
        }
    } else {
        None
    };
    
    info!("Copying file from {} to {}", src, dest);
    
    // Check if source file exists
    let src_path = Path::new(&src);
    if !src_path.exists() {
        return Err(anyhow::anyhow!("Source file does not exist: {}", src));
    }
    
    // Perform the file copy
    ssh_client.upload_file(&src, &dest)
        .context(format!("Failed to copy file from {} to {}", src, dest))?;
    
    // Set file mode if specified
    if let Some(mode_str) = mode {
        set_file_mode(ssh_client, &dest, &mode_str, use_become, become_user)?;
    }
    
    // Set ownership if specified
    if owner.is_some() || group.is_some() {
        set_ownership(ssh_client, &dest, owner.as_deref(), group.as_deref(), use_become, become_user)?;
    }
    
    info!("File copied successfully");
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

pub fn execute_adhoc(host: &Host, copy_args: &Value) -> Result<ModuleResult> {
    info!("Connecting to host: {}", host.name);
    let ssh_client = SshClient::connect(host)?;
    
    // 解析源文件和目标文件
    let src_file = get_param(copy_args, "src")?;
    let dest_file = get_param(copy_args, "dest")?;
    
    execute(&ssh_client, copy_args, false, "")?;
    
    Ok(ModuleResult {
        stdout: String::new(),
        stderr: String::new(),
        changed: true,
        msg: format!("Copied {} to {}", src_file, dest_file),
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