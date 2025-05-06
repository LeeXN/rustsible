use anyhow::{Result, Context};
use log::info;
use serde_yaml::Value;
use std::path::Path;

use crate::inventory::Host;
use crate::ssh::connection::SshClient;
use crate::modules::ModuleResult;
use crate::modules::param::{get_param, get_optional_param};
use crate::modules::remote::{set_file_mode, set_ownership};

/// Execute the copy module logic: upload a file and set permissions/ownership if needed.
pub fn execute(ssh_client: &SshClient, copy_args: &Value, use_become: bool, become_user: &str) -> Result<()> {
    let src = get_param::<String>(copy_args, "src")?;
    let dest = get_param::<String>(copy_args, "dest")?;
    let mode = get_optional_param::<String>(copy_args, "mode");
    let owner = get_optional_param::<String>(copy_args, "owner");
    let group = get_optional_param::<String>(copy_args, "group");
    
    info!("Copying file from {} to {}", src, dest);
    // Check if source file exists locally
    let src_path = Path::new(&src);
    if !src_path.exists() {
        return Err(anyhow::anyhow!("Source file does not exist: {}", src));
    }
    // Upload the file
    ssh_client.upload_file(&src, &dest)
        .context(format!("Failed to copy file from {} to {}", src, dest))?;
    // Set file mode if specified
    if let Some(mode_str) = mode.as_deref() {
        set_file_mode(ssh_client, &dest, mode_str, use_become, become_user)?;
    }
    // Set ownership if specified
    if owner.is_some() || group.is_some() {
        set_ownership(ssh_client, &dest, owner.as_deref(), group.as_deref(), use_become, become_user)?;
    }
    info!("File copied successfully");
    Ok(())
}

/// Execute the copy module in ad-hoc mode for a single host.
pub fn execute_adhoc(host: &Host, copy_args: &Value) -> Result<ModuleResult> {
    info!("Connecting to host: {}", host.name);
    let ssh_client = SshClient::connect(host)?;
    let src_file = get_param::<String>(copy_args, "src")?;
    let dest_file = get_param::<String>(copy_args, "dest")?;
    execute(&ssh_client, copy_args, false, "")?;
    Ok(ModuleResult {
        stdout: String::new(),
        stderr: String::new(),
        changed: true,
        msg: format!("Copied {} to {}", src_file, dest_file),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::{Value, Mapping};

    #[test]
    fn test_copy_param_extract_ok() {
        let mut map = Mapping::new();
        map.insert(Value::String("src".to_string()), Value::String("/tmp/a".to_string()));
        map.insert(Value::String("dest".to_string()), Value::String("/tmp/b".to_string()));
        let args = Value::Mapping(map);
        assert_eq!(crate::modules::param::get_param::<String>(&args, "src").unwrap(), "/tmp/a");
        assert_eq!(crate::modules::param::get_param::<String>(&args, "dest").unwrap(), "/tmp/b");
    }

    #[test]
    fn test_copy_param_missing() {
        let map = Mapping::new();
        let args = Value::Mapping(map);
        assert!(crate::modules::param::get_param::<String>(&args, "src").is_err());
    }
} 