use anyhow::{Result, Context};
use log::info;
use serde_yaml::Value;
use std::process::Command;
use uuid::Uuid;

use crate::inventory::Host;
use crate::ssh::connection::SshClient;
use crate::modules::ModuleResult;
use crate::modules::param::{get_param, get_optional_param};

/// Execute the user module logic: manage user accounts
pub fn execute(ssh_client: &SshClient, args: &Value, use_become: bool, _become_user: &str) -> Result<ModuleResult> {
    let name = get_param::<String>(args, "name")?;
    
    // Extract parameters
    let state = get_optional_param::<String>(args, "state").unwrap_or_else(|| "present".to_string());
    let uid = get_optional_param::<i64>(args, "uid");
    let gid = get_optional_param::<i64>(args, "gid");
    let groups = get_optional_param::<Vec<String>>(args, "groups");
    let append = get_optional_param::<bool>(args, "append").unwrap_or(false);
    let home = get_optional_param::<String>(args, "home");
    let shell = get_optional_param::<String>(args, "shell");
    let comment = get_optional_param::<String>(args, "comment");
    let password = get_optional_param::<String>(args, "password");
    let create_home = get_optional_param::<bool>(args, "create_home").unwrap_or(true);
    let system = get_optional_param::<bool>(args, "system").unwrap_or(false);
    let remove = get_optional_param::<bool>(args, "remove").unwrap_or(false);
    
    info!("Managing user: {}", name);
    
    // Check if we need _host_type for local execution
    let is_local = if let Value::Mapping(args_map) = args {
        if let Some(Value::String(host_type)) = args_map.get(&Value::String("_host_type".to_string())) {
            host_type == "local"
        } else {
            false
        }
    } else {
        false
    };
    
    if is_local {
        execute_local(&name, &state, uid, gid, groups, append, home, shell, comment, password, create_home, system, remove)
    } else {
        execute_remote(ssh_client, &name, &state, uid, gid, groups, append, home, shell, comment, password, create_home, system, remove, use_become)
    }
}

/// Execute user management locally
fn execute_local(
    name: &str,
    state: &str,
    uid: Option<i64>,
    gid: Option<i64>,
    groups: Option<Vec<String>>,
    append: bool,
    home: Option<String>,
    shell: Option<String>,
    comment: Option<String>,
    password: Option<String>,
    create_home: bool,
    system: bool,
    remove: bool,
) -> Result<ModuleResult> {
    let user_exists = check_user_exists_local(name)?;
    let mut changed = false;
    let mut msg;
    
    match state {
        "present" => {
            if !user_exists {
                // Create user
                create_user_local(name, uid, gid, home.as_deref(), shell.as_deref(), comment.as_deref(), create_home, system)?;
                changed = true;
                msg = format!("User {} created", name);
            } else {
                // Modify existing user
                let modify_result = modify_user_local(name, uid, gid, home.as_deref(), shell.as_deref(), comment.as_deref())?;
                changed = modify_result;
                msg = if changed {
                    format!("User {} modified", name)
                } else {
                    format!("User {} already exists with correct configuration", name)
                };
            }
            
            // Handle password if provided
            if let Some(password_hash) = password {
                set_user_password_local(name, &password_hash)?;
                if !changed {
                    changed = true;
                    msg = format!("User {} password updated", name);
                }
            }
            
            // Handle groups if provided
            if let Some(group_list) = groups {
                manage_user_groups_local(name, &group_list, append)?;
                if !changed {
                    changed = true;
                    msg = format!("User {} groups updated", name);
                }
            }
        },
        "absent" => {
            if user_exists {
                remove_user_local(name, remove)?;
                changed = true;
                msg = format!("User {} removed", name);
            } else {
                msg = format!("User {} already absent", name);
            }
        },
        _ => {
            return Err(anyhow::anyhow!("Invalid state: {}. Must be 'present' or 'absent'", state));
        }
    }
    
    Ok(ModuleResult {
        stdout: String::new(),
        stderr: String::new(),
        changed,
        msg,
    })
}

/// Execute user management remotely via SSH
fn execute_remote(
    ssh_client: &SshClient,
    name: &str,
    state: &str,
    uid: Option<i64>,
    gid: Option<i64>,
    groups: Option<Vec<String>>,
    append: bool,
    home: Option<String>,
    shell: Option<String>,
    comment: Option<String>,
    password: Option<String>,
    create_home: bool,
    system: bool,
    remove: bool,
    use_become: bool,
) -> Result<ModuleResult> {
    let user_exists = check_user_exists_remote(ssh_client, name, use_become)?;
    let mut changed = false;
    let mut msg;
    
    match state {
        "present" => {
            if !user_exists {
                // Create user
                create_user_remote(ssh_client, name, uid, gid, home.as_deref(), shell.as_deref(), comment.as_deref(), create_home, system, use_become)?;
                changed = true;
                msg = format!("User {} created", name);
            } else {
                // Modify existing user
                let modify_result = modify_user_remote(ssh_client, name, uid, gid, home.as_deref(), shell.as_deref(), comment.as_deref(), use_become)?;
                changed = modify_result;
                msg = if changed {
                    format!("User {} modified", name)
                } else {
                    format!("User {} already exists with correct configuration", name)
                };
            }
            
            // Handle password if provided
            if let Some(password_hash) = password {
                set_user_password_remote(ssh_client, name, &password_hash, use_become)?;
                if !changed {
                    changed = true;
                    msg = format!("User {} password updated", name);
                }
            }
            
            // Handle groups if provided
            if let Some(group_list) = groups {
                manage_user_groups_remote(ssh_client, name, &group_list, append, use_become)?;
                if !changed {
                    changed = true;
                    msg = format!("User {} groups updated", name);
                }
            }
        },
        "absent" => {
            if user_exists {
                remove_user_remote(ssh_client, name, remove, use_become)?;
                changed = true;
                msg = format!("User {} removed", name);
            } else {
                msg = format!("User {} already absent", name);
            }
        },
        _ => {
            return Err(anyhow::anyhow!("Invalid state: {}. Must be 'present' or 'absent'", state));
        }
    }
    
    Ok(ModuleResult {
        stdout: String::new(),
        stderr: String::new(),
        changed,
        msg,
    })
}

/// Check if user exists locally
fn check_user_exists_local(name: &str) -> Result<bool> {
    let output = Command::new("id")
        .arg(name)
        .output()
        .with_context(|| format!("Failed to check if user {} exists", name))?;
    
    Ok(output.status.success())
}

/// Check if user exists remotely
fn check_user_exists_remote(ssh_client: &SshClient, name: &str, use_become: bool) -> Result<bool> {
    let cmd = format!("id {}", name);
    let (exit_code, _, _) = if use_become {
        ssh_client.execute_sudo_command(&cmd, "")?
    } else {
        ssh_client.execute_command(&cmd)?
    };
    
    Ok(exit_code == 0)
}

/// Create user locally
fn create_user_local(
    name: &str,
    uid: Option<i64>,
    gid: Option<i64>,
    home: Option<&str>,
    shell: Option<&str>,
    comment: Option<&str>,
    create_home: bool,
    system: bool,
) -> Result<()> {
    let mut cmd = Command::new("useradd");
    
    if let Some(uid_val) = uid {
        cmd.args(&["--uid", &uid_val.to_string()]);
    }
    
    if let Some(gid_val) = gid {
        cmd.args(&["--gid", &gid_val.to_string()]);
    }
    
    if let Some(home_dir) = home {
        cmd.args(&["--home-dir", home_dir]);
    }
    
    if let Some(shell_path) = shell {
        cmd.args(&["--shell", shell_path]);
    }
    
    if let Some(comment_text) = comment {
        cmd.args(&["--comment", comment_text]);
    }
    
    if create_home {
        cmd.arg("--create-home");
    } else {
        cmd.arg("--no-create-home");
    }
    
    if system {
        cmd.arg("--system");
    }
    
    cmd.arg(name);
    
    let output = cmd.output()
        .with_context(|| format!("Failed to create user {}", name))?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("Failed to create user {}: {}", name, stderr));
    }
    
    Ok(())
}

/// Create user remotely
fn create_user_remote(
    ssh_client: &SshClient,
    name: &str,
    uid: Option<i64>,
    gid: Option<i64>,
    home: Option<&str>,
    shell: Option<&str>,
    comment: Option<&str>,
    create_home: bool,
    system: bool,
    use_become: bool,
) -> Result<()> {
    let mut cmd = vec!["useradd".to_string()];
    
    if let Some(uid_val) = uid {
        cmd.push("--uid".to_string());
        cmd.push(uid_val.to_string());
    }
    
    if let Some(gid_val) = gid {
        cmd.push("--gid".to_string());
        cmd.push(gid_val.to_string());
    }
    
    if let Some(home_dir) = home {
        cmd.push("--home-dir".to_string());
        cmd.push(home_dir.to_string());
    }
    
    if let Some(shell_path) = shell {
        cmd.push("--shell".to_string());
        cmd.push(shell_path.to_string());
    }
    
    if let Some(comment_text) = comment {
        cmd.push("--comment".to_string());
        cmd.push(format!("\"{}\"", comment_text));
    }
    
    if create_home {
        cmd.push("--create-home".to_string());
    } else {
        cmd.push("--no-create-home".to_string());
    }
    
    if system {
        cmd.push("--system".to_string());
    }
    
    cmd.push(name.to_string());
    
    let full_cmd = cmd.join(" ");
    let (exit_code, _, stderr) = if use_become {
        ssh_client.execute_sudo_command(&full_cmd, "")?
    } else {
        ssh_client.execute_command(&full_cmd)?
    };
    
    if exit_code != 0 {
        return Err(anyhow::anyhow!("Failed to create user {}: {}", name, stderr));
    }
    
    Ok(())
}

/// Modify user locally
fn modify_user_local(
    name: &str,
    uid: Option<i64>,
    gid: Option<i64>,
    home: Option<&str>,
    shell: Option<&str>,
    comment: Option<&str>,
) -> Result<bool> {
    let mut changed = false;
    let mut cmd = Command::new("usermod");
    let mut has_changes = false;
    
    if let Some(uid_val) = uid {
        cmd.args(&["--uid", &uid_val.to_string()]);
        has_changes = true;
    }
    
    if let Some(gid_val) = gid {
        cmd.args(&["--gid", &gid_val.to_string()]);
        has_changes = true;
    }
    
    if let Some(home_dir) = home {
        cmd.args(&["--home", home_dir]);
        has_changes = true;
    }
    
    if let Some(shell_path) = shell {
        cmd.args(&["--shell", shell_path]);
        has_changes = true;
    }
    
    if let Some(comment_text) = comment {
        cmd.args(&["--comment", comment_text]);
        has_changes = true;
    }
    
    if has_changes {
        cmd.arg(name);
        
        let output = cmd.output()
            .with_context(|| format!("Failed to modify user {}", name))?;
        
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Failed to modify user {}: {}", name, stderr));
        }
        
        changed = true;
    }
    
    Ok(changed)
}

/// Modify user remotely
fn modify_user_remote(
    ssh_client: &SshClient,
    name: &str,
    uid: Option<i64>,
    gid: Option<i64>,
    home: Option<&str>,
    shell: Option<&str>,
    comment: Option<&str>,
    use_become: bool,
) -> Result<bool> {
    let mut cmd = vec!["usermod".to_string()];
    let mut has_changes = false;
    
    if let Some(uid_val) = uid {
        cmd.push("--uid".to_string());
        cmd.push(uid_val.to_string());
        has_changes = true;
    }
    
    if let Some(gid_val) = gid {
        cmd.push("--gid".to_string());
        cmd.push(gid_val.to_string());
        has_changes = true;
    }
    
    if let Some(home_dir) = home {
        cmd.push("--home".to_string());
        cmd.push(home_dir.to_string());
        has_changes = true;
    }
    
    if let Some(shell_path) = shell {
        cmd.push("--shell".to_string());
        cmd.push(shell_path.to_string());
        has_changes = true;
    }
    
    if let Some(comment_text) = comment {
        cmd.push("--comment".to_string());
        cmd.push(format!("\"{}\"", comment_text));
        has_changes = true;
    }
    
    if has_changes {
        cmd.push(name.to_string());
        
        let full_cmd = cmd.join(" ");
        let (exit_code, _, stderr) = if use_become {
            ssh_client.execute_sudo_command(&full_cmd, "")?
        } else {
            ssh_client.execute_command(&full_cmd)?
        };
        
        if exit_code != 0 {
            return Err(anyhow::anyhow!("Failed to modify user {}: {}", name, stderr));
        }
        
        return Ok(true);
    }
    
    Ok(false)
}

/// Set user password locally
fn set_user_password_local(name: &str, password_hash: &str) -> Result<()> {
    // Use sudo directly for local operations as well to ensure proper permissions
    let cmd = format!("echo '{}:{}' | sudo chpasswd -e", name, password_hash);
    
    let output = Command::new("sh")
        .args(&["-c", &cmd])
        .output()
        .with_context(|| format!("Failed to set password for user {}", name))?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("Failed to set password for user {}: {}", name, stderr));
    }
    
    Ok(())
}

/// Set user password remotely
fn set_user_password_remote(
    ssh_client: &SshClient,
    name: &str,
    password_hash: &str,
    use_become: bool,
) -> Result<()> {
    // Use a different approach for setting passwords with sudo
    // Instead of using pipe with sudo, use usermod command which works better with sudo
    let cmd = if use_become {
        // Use usermod --password instead of chpasswd for better sudo compatibility
        format!("usermod --password '{}' {}", password_hash, name)
    } else {
        // For non-sudo, use the original approach
        format!("echo '{}:{}' | chpasswd -e", name, password_hash)
    };
    
    let (exit_code, _, stderr) = if use_become {
        ssh_client.execute_sudo_command(&cmd, "")?
    } else {
        ssh_client.execute_command(&cmd)?
    };
    
    if exit_code != 0 {
        // If usermod fails, try alternative approach
        if use_become && cmd.contains("usermod") {
            info!("usermod password failed, trying alternative method for user {}", name);
            
            // Alternative: Create a temporary script and execute it with sudo
            let script_content = format!("#!/bin/bash\necho '{}:{}' | chpasswd -e", name, password_hash);
            let script_path = format!("/tmp/set_password_{}.sh", Uuid::new_v4());
            
            // Create the script file
            let create_script_cmd = format!("cat > {} << 'EOF'\n{}\nEOF", script_path, script_content);
            let (create_exit, _, create_stderr) = ssh_client.execute_command(&create_script_cmd)?;
            
            if create_exit != 0 {
                return Err(anyhow::anyhow!("Failed to create password script for user {}: {}", name, create_stderr));
            }
            
            // Make it executable
            let chmod_cmd = format!("chmod +x {}", script_path);
            let (chmod_exit, _, chmod_stderr) = ssh_client.execute_command(&chmod_cmd)?;
            
            if chmod_exit != 0 {
                // Clean up and return error
                let _ = ssh_client.execute_command(&format!("rm -f {}", script_path));
                return Err(anyhow::anyhow!("Failed to make password script executable for user {}: {}", name, chmod_stderr));
            }
            
            // Execute the script with sudo
            let exec_cmd = format!("bash {}", script_path);
            let (exec_exit, _, exec_stderr) = ssh_client.execute_sudo_command(&exec_cmd, "")?;
            
            // Clean up the script
            let _ = ssh_client.execute_command(&format!("rm -f {}", script_path));
            
            if exec_exit != 0 {
                return Err(anyhow::anyhow!("Failed to set password for user {} using script method: {}", name, exec_stderr));
            }
            
            info!("Successfully set password for user {} using alternative method", name);
            return Ok(());
        }
        
        return Err(anyhow::anyhow!("Failed to set password for user {}: {}", name, stderr));
    }
    
    Ok(())
}

/// Manage user groups locally
fn manage_user_groups_local(name: &str, groups: &[String], append: bool) -> Result<()> {
    let mut cmd = Command::new("usermod");
    
    if append {
        cmd.args(&["-a", "-G"]);
    } else {
        cmd.args(&["-G"]);
    }
    
    cmd.arg(groups.join(","));
    cmd.arg(name);
    
    let output = cmd.output()
        .with_context(|| format!("Failed to manage groups for user {}", name))?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("Failed to manage groups for user {}: {}", name, stderr));
    }
    
    Ok(())
}

/// Manage user groups remotely
fn manage_user_groups_remote(
    ssh_client: &SshClient,
    name: &str,
    groups: &[String],
    append: bool,
    use_become: bool,
) -> Result<()> {
    let cmd = if append {
        format!("usermod -a -G {} {}", groups.join(","), name)
    } else {
        format!("usermod -G {} {}", groups.join(","), name)
    };
    
    let (exit_code, _, stderr) = if use_become {
        ssh_client.execute_sudo_command(&cmd, "")?
    } else {
        ssh_client.execute_command(&cmd)?
    };
    
    if exit_code != 0 {
        return Err(anyhow::anyhow!("Failed to manage groups for user {}: {}", name, stderr));
    }
    
    Ok(())
}

/// Remove user locally
fn remove_user_local(name: &str, remove_home: bool) -> Result<()> {
    let mut cmd = Command::new("userdel");
    
    if remove_home {
        cmd.arg("--remove");
    }
    
    cmd.arg(name);
    
    let output = cmd.output()
        .with_context(|| format!("Failed to remove user {}", name))?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("Failed to remove user {}: {}", name, stderr));
    }
    
    Ok(())
}

/// Remove user remotely
fn remove_user_remote(
    ssh_client: &SshClient,
    name: &str,
    remove_home: bool,
    use_become: bool,
) -> Result<()> {
    let cmd = if remove_home {
        format!("userdel --remove {}", name)
    } else {
        format!("userdel {}", name)
    };
    
    let (exit_code, _, stderr) = if use_become {
        ssh_client.execute_sudo_command(&cmd, "")?
    } else {
        ssh_client.execute_command(&cmd)?
    };
    
    if exit_code != 0 {
        return Err(anyhow::anyhow!("Failed to remove user {}: {}", name, stderr));
    }
    
    Ok(())
}

/// Execute the user module in ad-hoc mode for a single host.
pub fn execute_adhoc(host: &Host, args: &Value) -> Result<ModuleResult> {
    if host.hostname == "localhost" || host.hostname == "127.0.0.1" {
        // For localhost, execute directly without SSH
        let name = get_param::<String>(args, "name")?;
        let state = get_optional_param::<String>(args, "state").unwrap_or_else(|| "present".to_string());
        let uid = get_optional_param::<i64>(args, "uid");
        let gid = get_optional_param::<i64>(args, "gid");
        let groups = get_optional_param::<Vec<String>>(args, "groups");
        let append = get_optional_param::<bool>(args, "append").unwrap_or(false);
        let home = get_optional_param::<String>(args, "home");
        let shell = get_optional_param::<String>(args, "shell");
        let comment = get_optional_param::<String>(args, "comment");
        let password = get_optional_param::<String>(args, "password");
        let create_home = get_optional_param::<bool>(args, "create_home").unwrap_or(true);
        let system = get_optional_param::<bool>(args, "system").unwrap_or(false);
        let remove = get_optional_param::<bool>(args, "remove").unwrap_or(false);
        
        return execute_local(&name, &state, uid, gid, groups, append, home, shell, comment, password, create_home, system, remove);
    }
    
    info!("Connecting to host: {}", host.name);
    let ssh_client = SshClient::connect(host)?;
    execute(&ssh_client, args, false, "")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::{Value, Mapping};

    #[test]
    fn test_check_user_exists_local() {
        // Test with a user that should exist on most systems
        let result = check_user_exists_local("root");
        assert!(result.is_ok());
        
        // Test with a user that should not exist
        let result = check_user_exists_local("nonexistent_user_12345");
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn test_user_module_params() {
        let mut map = Mapping::new();
        map.insert(Value::String("name".to_string()), Value::String("testuser".to_string()));
        map.insert(Value::String("state".to_string()), Value::String("present".to_string()));
        let args = Value::Mapping(map);
        
        assert_eq!(get_param::<String>(&args, "name").unwrap(), "testuser");
        assert_eq!(get_optional_param::<String>(&args, "state").unwrap(), "present");
    }
} 