use anyhow::{Result, Context};
use log::{info, warn};
use serde_yaml::Value;
use std::fs;
use regex::Regex;

use crate::inventory::Host;
use crate::ssh::connection::SshClient;
use crate::modules::ModuleResult;
use crate::modules::param::{get_param, get_optional_param};

/// Execute the lineinfile module logic: manage lines in a file
pub fn execute(ssh_client: &SshClient, args: &Value, use_become: bool, _become_user: &str) -> Result<ModuleResult> {
    let path = get_param::<String>(args, "path")?;
    
    // Extract parameters
    let line = get_optional_param::<String>(args, "line");
    let regexp = get_optional_param::<String>(args, "regexp");
    let state = get_optional_param::<String>(args, "state").unwrap_or_else(|| "present".to_string());
    let backup = get_optional_param::<bool>(args, "backup").unwrap_or(false);
    let create = get_optional_param::<bool>(args, "create").unwrap_or(false);
    let insertafter = get_optional_param::<String>(args, "insertafter");
    let insertbefore = get_optional_param::<String>(args, "insertbefore");
    let owner = get_optional_param::<String>(args, "owner");
    let group = get_optional_param::<String>(args, "group");
    let mode = get_optional_param::<String>(args, "mode");

    info!("Managing line in file: {}", path);
    
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
        execute_local(&path, line, regexp, &state, backup, create, insertafter, insertbefore, owner, group, mode)
    } else {
        execute_remote(ssh_client, &path, line, regexp, &state, backup, create, insertafter, insertbefore, owner, group, mode, use_become)
    }
}

/// Execute lineinfile locally
fn execute_local(
    path: &str,
    line: Option<String>,
    regexp: Option<String>,
    state: &str,
    backup: bool,
    create: bool,
    insertafter: Option<String>,
    insertbefore: Option<String>,
    owner: Option<String>,
    group: Option<String>,
    mode: Option<String>,
) -> Result<ModuleResult> {
    let path_obj = std::path::Path::new(path);
    
    // Check if file exists
    let file_exists = path_obj.exists();
    
    if !file_exists && !create {
        return Err(anyhow::anyhow!("File {} does not exist and create=false", path));
    }
    
    let mut content = if file_exists {
        fs::read_to_string(path)
            .with_context(|| format!("Failed to read file: {}", path))?
    } else {
        String::new()
    };
    
    let _original_content = content.clone();
    
    // Create backup if requested
    if backup && file_exists {
        let backup_path = format!("{}.backup", path);
        fs::copy(path, &backup_path)
            .with_context(|| format!("Failed to create backup: {}", backup_path))?;
        info!("Created backup: {}", backup_path);
    }
    
    let result = process_line_modifications(&mut content, line, regexp, state, insertafter, insertbefore)?;
    let changed = result;
    
    if changed || !file_exists {
        // Write the file
        fs::write(path, &content)
            .with_context(|| format!("Failed to write file: {}", path))?;
        
        // Set permissions if specified
        if let Some(mode_str) = mode {
            set_file_permissions(path, &mode_str)?;
        }
        
        // Set ownership if specified (Unix only)
        #[cfg(unix)]
        if owner.is_some() || group.is_some() {
            set_file_ownership(path, owner.as_deref(), group.as_deref())?;
        }
    }
    
    let msg = if !file_exists {
        format!("File {} created", path)
    } else if changed {
        format!("Line {} in {}", if state == "present" { "added/updated" } else { "removed" }, path)
    } else {
        format!("File {} unchanged", path)
    };
    
    Ok(ModuleResult {
        stdout: String::new(),
        stderr: String::new(),
        changed: changed || !file_exists,
        msg,
    })
}

/// Execute lineinfile remotely via SSH
fn execute_remote(
    ssh_client: &SshClient,
    path: &str,
    line: Option<String>,
    regexp: Option<String>,
    state: &str,
    backup: bool,
    create: bool,
    insertafter: Option<String>,
    insertbefore: Option<String>,
    owner: Option<String>,
    group: Option<String>,
    mode: Option<String>,
    use_become: bool,
) -> Result<ModuleResult> {
    // Check if file exists
    let check_cmd = format!("test -f {}", path);
    let (exit_code, _, _) = if use_become {
        ssh_client.execute_sudo_command(&check_cmd, "")?
    } else {
        ssh_client.execute_command(&check_cmd)?
    };
    
    let file_exists = exit_code == 0;
    
    if !file_exists && !create {
        return Err(anyhow::anyhow!("File {} does not exist and create=false", path));
    }
    
    // Read file content if it exists
    let mut content = if file_exists {
        let read_cmd = format!("cat {}", path);
        let (exit_code, stdout, stderr) = if use_become {
            ssh_client.execute_sudo_command(&read_cmd, "")?
        } else {
            ssh_client.execute_command(&read_cmd)?
        };
        
        if exit_code != 0 {
            return Err(anyhow::anyhow!("Failed to read file {}: {}", path, stderr));
        }
        stdout
    } else {
        String::new()
    };
    
    let _original_content = content.clone();
    
    // Create backup if requested
    if backup && file_exists {
        let backup_cmd = format!("cp {} {}.backup", path, path);
        let (exit_code, _, stderr) = if use_become {
            ssh_client.execute_sudo_command(&backup_cmd, "")?
        } else {
            ssh_client.execute_command(&backup_cmd)?
        };
        
        if exit_code != 0 {
            warn!("Failed to create backup: {}", stderr);
        } else {
            info!("Created backup: {}.backup", path);
        }
    }
    
    let changed = process_line_modifications(&mut content, line, regexp, state, insertafter, insertbefore)?;
    
    if changed || !file_exists {
        // Write the modified content to the file
        if use_become {
            ssh_client.write_file_with_sudo(
                &content,
                path,
                mode.as_deref(),
                owner.as_deref(),
                group.as_deref(),
            )?;
        } else {
            ssh_client.write_file_content(path, &content)?;
            
            // Set permissions and ownership if specified (without sudo)
            if let Some(mode_str) = mode {
                let chmod_cmd = format!("chmod {} {}", mode_str, path);
                let (exit_code, _, stderr) = ssh_client.execute_command(&chmod_cmd)?;
                if exit_code != 0 {
                    return Err(anyhow::anyhow!("Failed to set file mode: {}", stderr));
                }
            }
            
            if owner.is_some() || group.is_some() {
                let ownership = match (owner.as_deref(), group.as_deref()) {
                    (Some(o), Some(g)) => format!("{}:{}", o, g),
                    (Some(o), None) => o.to_string(),
                    (None, Some(g)) => format!(":{}", g),
                    (None, None) => String::new(),
                };
                
                if !ownership.is_empty() {
                    let chown_cmd = format!("chown {} {}", ownership, path);
                    let (exit_code, _, stderr) = ssh_client.execute_command(&chown_cmd)?;
                    if exit_code != 0 {
                        return Err(anyhow::anyhow!("Failed to set file ownership: {}", stderr));
                    }
                }
            }
        }
    }
    
    let msg = if !file_exists {
        format!("File {} created", path)
    } else if changed {
        format!("Line {} in {}", if state == "present" { "added/updated" } else { "removed" }, path)
    } else {
        format!("File {} unchanged", path)
    };
    
    Ok(ModuleResult {
        stdout: String::new(),
        stderr: String::new(),
        changed: changed || !file_exists,
        msg,
    })
}

/// Process line modifications to content
fn process_line_modifications(
    content: &mut String,
    line: Option<String>,
    regexp: Option<String>,
    state: &str,
    insertafter: Option<String>,
    insertbefore: Option<String>,
) -> Result<bool> {
    let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let mut changed = false;
    
    match state {
        "present" => {
            let line_to_add = line.as_ref()
                .ok_or_else(|| anyhow::anyhow!("'line' parameter is required when state=present"))?;
            
            if let Some(regex_pattern) = regexp {
                // Find and replace line matching regexp
                let regex = Regex::new(&regex_pattern)
                    .with_context(|| format!("Invalid regexp: {}", regex_pattern))?;
                
                let mut found = false;
                for line in lines.iter_mut() {
                    if regex.is_match(line) {
                        if line != line_to_add {
                            *line = line_to_add.clone();
                            changed = true;
                        }
                        found = true;
                        break;
                    }
                }
                
                if !found {
                    // Line doesn't exist, add it
                    insert_line(&mut lines, line_to_add, insertafter.as_deref(), insertbefore.as_deref())?;
                    changed = true;
                }
            } else {
                // Check if line already exists
                if !lines.iter().any(|existing_line| existing_line == line_to_add) {
                    insert_line(&mut lines, line_to_add, insertafter.as_deref(), insertbefore.as_deref())?;
                    changed = true;
                }
            }
        },
        "absent" => {
            if let Some(regex_pattern) = regexp {
                // Remove lines matching regexp
                let regex = Regex::new(&regex_pattern)
                    .with_context(|| format!("Invalid regexp: {}", regex_pattern))?;
                
                let original_len = lines.len();
                lines.retain(|line| !regex.is_match(line));
                changed = lines.len() != original_len;
            } else if let Some(line_to_remove) = line {
                // Remove specific line
                let original_len = lines.len();
                lines.retain(|line| *line != line_to_remove);
                changed = lines.len() != original_len;
            } else {
                return Err(anyhow::anyhow!("Either 'line' or 'regexp' parameter is required when state=absent"));
            }
        },
        _ => {
            return Err(anyhow::anyhow!("Invalid state: {}. Must be 'present' or 'absent'", state));
        }
    }
    
    *content = lines.join("\n");
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    
    Ok(changed)
}

/// Insert line at appropriate position
fn insert_line(
    lines: &mut Vec<String>,
    line_to_add: &str,
    insertafter: Option<&str>,
    insertbefore: Option<&str>,
) -> Result<()> {
    if let Some(pattern) = insertafter {
        if pattern == "EOF" {
            lines.push(line_to_add.to_string());
        } else {
            let regex = Regex::new(pattern)
                .with_context(|| format!("Invalid insertafter regexp: {}", pattern))?;
            
            let mut insert_pos = lines.len(); // Default to end
            for (i, line) in lines.iter().enumerate() {
                if regex.is_match(line) {
                    insert_pos = i + 1;
                    break;
                }
            }
            lines.insert(insert_pos, line_to_add.to_string());
        }
    } else if let Some(pattern) = insertbefore {
        if pattern == "BOF" {
            lines.insert(0, line_to_add.to_string());
        } else {
            let regex = Regex::new(pattern)
                .with_context(|| format!("Invalid insertbefore regexp: {}", pattern))?;
            
            let mut insert_pos = lines.len(); // Default to end
            for (i, line) in lines.iter().enumerate() {
                if regex.is_match(line) {
                    insert_pos = i;
                    break;
                }
            }
            lines.insert(insert_pos, line_to_add.to_string());
        }
    } else {
        lines.push(line_to_add.to_string());
    }
    
    Ok(())
}

/// Set file permissions (Unix only)
#[cfg(unix)]
fn set_file_permissions(path: &str, mode: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    
    let mode_value = u32::from_str_radix(mode, 8)
        .with_context(|| format!("Invalid file mode: {}", mode))?;
    
    let perms = std::fs::Permissions::from_mode(mode_value);
    std::fs::set_permissions(path, perms)
        .with_context(|| format!("Failed to set permissions on {}", path))?;
    
    Ok(())
}

/// Set file permissions (Windows - limited support)
#[cfg(windows)]
fn set_file_permissions(_path: &str, _mode: &str) -> Result<()> {
    warn!("File mode setting is not supported on Windows");
    Ok(())
}

/// Set file ownership (Unix only)
#[cfg(unix)]
fn set_file_ownership(path: &str, owner: Option<&str>, group: Option<&str>) -> Result<()> {
    use std::process::Command;
    
    if let Some(owner_name) = owner {
        let chown_cmd = if let Some(group_name) = group {
            format!("chown {}:{} {}", owner_name, group_name, path)
        } else {
            format!("chown {} {}", owner_name, path)
        };
        
        let output = Command::new("sh")
            .args(&["-c", &chown_cmd])
            .output()
            .with_context(|| format!("Failed to execute chown command: {}", chown_cmd))?;
        
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Failed to set ownership: {}", stderr));
        }
    } else if let Some(group_name) = group {
        let chgrp_cmd = format!("chgrp {} {}", group_name, path);
        
        let output = Command::new("sh")
            .args(&["-c", &chgrp_cmd])
            .output()
            .with_context(|| format!("Failed to execute chgrp command: {}", chgrp_cmd))?;
        
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Failed to set group ownership: {}", stderr));
        }
    }
    
    Ok(())
}

/// Execute the lineinfile module in ad-hoc mode for a single host.
pub fn execute_adhoc(host: &Host, args: &Value) -> Result<ModuleResult> {
    if host.hostname == "localhost" || host.hostname == "127.0.0.1" {
        // For localhost, execute directly without SSH
        let path = get_param::<String>(args, "path")?;
        let line = get_optional_param::<String>(args, "line");
        let regexp = get_optional_param::<String>(args, "regexp");
        let state = get_optional_param::<String>(args, "state").unwrap_or_else(|| "present".to_string());
        let backup = get_optional_param::<bool>(args, "backup").unwrap_or(false);
        let create = get_optional_param::<bool>(args, "create").unwrap_or(false);
        let insertafter = get_optional_param::<String>(args, "insertafter");
        let insertbefore = get_optional_param::<String>(args, "insertbefore");
        let owner = get_optional_param::<String>(args, "owner");
        let group = get_optional_param::<String>(args, "group");
        let mode = get_optional_param::<String>(args, "mode");
        
        return execute_local(&path, line, regexp, &state, backup, create, insertafter, insertbefore, owner, group, mode);
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
    fn test_process_line_modifications_present() {
        let mut content = "line1\nline2\nline3\n".to_string();
        let changed = process_line_modifications(
            &mut content,
            Some("new_line".to_string()),
            None,
            "present",
            None,
            None,
        ).unwrap();
        
        assert!(changed);
        assert!(content.contains("new_line"));
    }

    #[test]
    fn test_process_line_modifications_absent() {
        let mut content = "line1\nline2\nline3\n".to_string();
        let changed = process_line_modifications(
            &mut content,
            Some("line2".to_string()),
            None,
            "absent",
            None,
            None,
        ).unwrap();
        
        assert!(changed);
        assert!(!content.contains("line2"));
    }

    #[test]
    fn test_process_line_modifications_regexp() {
        let mut content = "config_option=old_value\nother_line\n".to_string();
        let changed = process_line_modifications(
            &mut content,
            Some("config_option=new_value".to_string()),
            Some("^config_option=".to_string()),
            "present",
            None,
            None,
        ).unwrap();
        
        assert!(changed);
        assert!(content.contains("config_option=new_value"));
        assert!(!content.contains("config_option=old_value"));
    }
} 