use anyhow::{Result, Context, anyhow};
use ssh2::Session;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use log::{debug, info, warn};
use uuid::Uuid;

use crate::inventory::Host;

pub struct SshClient {
    session: Session,
    host: String,
    sudo_password: String,
}

impl SshClient {
    pub fn connect(host: &Host) -> Result<Self> {
        info!("Connecting to host: {} ({}:{})", host.name, host.hostname, host.port);
        
        // 输出所有变量以便调试
        debug!("Host variables:");
        for (key, value) in &host.variables {
            debug!("  {} = {}", key, value);
        }
        
        // 显示将用于连接的重要变量
        if let Some(user) = host.get_ssh_user() {
            debug!("Using SSH user: {}", user);
        } else {
            debug!("No SSH user specified, using default: root");
        }
        
        if let Some(_) = host.get_ssh_password() {
            debug!("Password authentication available");
        } else {
            debug!("No password specified for authentication");
        }
        
        if let Some(key_path) = host.get_ssh_private_key() {
            debug!("SSH private key available: {}", key_path);
        } else {
            debug!("No SSH private key specified");
        }
        
        let tcp = TcpStream::connect(format!("{}:{}", host.hostname, host.port))
            .context(format!("Failed to connect to {}:{}", host.hostname, host.port))?;
        
        let mut session = Session::new().context("Failed to create SSH session")?;
        session.set_tcp_stream(tcp);
        debug!("Starting SSH handshake with {}", host.hostname);
        session.handshake().context("SSH handshake failed")?;
        
        // 获取用户名
        let username = host.get_ssh_user()
            .map(|s| s.as_str())
            .unwrap_or("root");
        // 获取sudo密码
        let sudo_password = host.get_ssh_sudo_password()
            .map(|s| s.as_str())
            .unwrap_or("");
        
        debug!("Using SSH username: {}", username);
        let mut auth_succeeded = false;
        
        // 尝试密码认证（ansible_ssh_pass）
        if let Some(password) = host.get_ssh_password() {
            debug!("Attempting password authentication for user {}", username);
            match session.userauth_password(username, password) {
                Ok(_) => {
                    info!("Password authentication succeeded for {}", username);
                    auth_succeeded = true;
                },
                Err(e) => {
                    warn!("Password authentication failed: {}", e);
                }
            }
        }
        
        // 如果密码认证失败，尝试私钥认证
        if !auth_succeeded {
            if let Some(key_path) = host.get_ssh_private_key() {
                let path = Path::new(key_path);
                debug!("Attempting private key authentication with key: {}", key_path);
                match session.userauth_pubkey_file(username, None, path, None) {
                    Ok(_) => {
                        info!("Private key authentication succeeded for {}", username);
                        auth_succeeded = true;
                    },
                    Err(e) => {
                        warn!("Private key authentication failed: {}", e);
                    }
                }
            }
        }
        
        // 如果仍未认证，尝试SSH代理认证
        if !auth_succeeded {
            debug!("Attempting SSH agent authentication for user {}", username);
            match session.userauth_agent(username) {
                Ok(_) => {
                    info!("SSH agent authentication succeeded for {}", username);
                    auth_succeeded = true;
                },
                Err(e) => {
                    warn!("SSH agent authentication failed: {}", e);
                }
            }
        }
        
        // 如果所有认证方法都失败，返回错误
        if !auth_succeeded {
            return Err(anyhow!("All authentication methods failed for {}@{}", username, host.hostname));
        }
        
        Ok(SshClient { 
            session,
            host: host.name.clone(),
            sudo_password: sudo_password.to_string(),
        })
    }
    
    pub fn execute_command(&self, command: &str) -> Result<(i32, String, String)> {
        let command_without_password = command.replace(&self.sudo_password, "SUDO-PASSWORD");
        debug!("Executing command on {}: {}", self.host, command_without_password);
        
        let mut channel = self.session.channel_session()
            .context("Failed to open SSH channel")?;
        
        channel.exec(command).context(format!("Failed to execute command: {}", command_without_password))?;
        
        let mut stdout = String::new();
        channel.read_to_string(&mut stdout).context("Failed to read command stdout")?;
        
        let mut stderr = String::new();
        channel.stderr().read_to_string(&mut stderr).context("Failed to read command stderr")?;
        
        channel.wait_close().context("Failed to wait for command completion")?;
        let exit_status = channel.exit_status().context("Failed to get command exit status")?;
        
        debug!("Command completed with exit code: {}", exit_status);
        
        Ok((exit_status, stdout, stderr))
    }
    
    pub fn execute_sudo_command(&self, command: &str, sudo_user: &str) -> Result<(i32, String, String)> {
        let sudo_cmd = if sudo_user.is_empty() || sudo_user == "root" {
            format!("echo \"{}\" | sudo -S {}", self.sudo_password, command)
        } else {
            format!("echo \"{}\" | sudo -S -u {} {}", self.sudo_password, sudo_user, command)
        };
        
        debug!("Executing sudo command on {}: sudo {}", self.host, command);
        self.execute_command(&sudo_cmd)
    }

    /// Write content to a file with sudo privileges using a temporary file approach
    pub fn write_file_with_sudo(&self, content: &str, remote_path: &str, mode: Option<&str>, owner: Option<&str>, group: Option<&str>) -> Result<()> {
        // Generate a unique temporary file name
        let temp_filename = format!("/tmp/rustsible_temp_{}", Uuid::new_v4().simple());
        
        debug!("Writing content to {} via temporary file {}", remote_path, temp_filename);
        
        // Step 1: Check if target file exists and get its current permissions
        let check_cmd = format!("test -f {}", remote_path);
        let (file_exists_code, _, _) = self.execute_sudo_command(&check_cmd, "")?;
        let file_exists = file_exists_code == 0;
        
        let (original_mode, original_owner, original_group) = if file_exists {
            // Get original file permissions and ownership
            let stat_cmd = format!("stat -c '%a %U %G' {}", remote_path);
            let (stat_exit_code, stat_output, stat_stderr) = self.execute_sudo_command(&stat_cmd, "")?;
            
            if stat_exit_code == 0 {
                let parts: Vec<&str> = stat_output.trim().split_whitespace().collect();
                if parts.len() >= 3 {
                    debug!("Original file permissions: mode={}, owner={}, group={}", parts[0], parts[1], parts[2]);
                    (Some(parts[0].to_string()), Some(parts[1].to_string()), Some(parts[2].to_string()))
                } else {
                    warn!("Failed to parse stat output: {}", stat_output);
                    (None, None, None)
                }
            } else {
                warn!("Failed to get file stats: {}", stat_stderr);
                (None, None, None)
            }
        } else {
            (None, None, None)
        };
        
        // Step 2: Write content to temporary file and set appropriate permissions
        self.write_file_content(&temp_filename, content)?;
        
        // Set the temporary file to have the same permissions as target (or default 644 for new files)
        let temp_mode = mode
            .map(|m| m.to_string())
            .or(original_mode.clone())
            .unwrap_or_else(|| "644".to_string());
            
        let chmod_temp_cmd = format!("chmod {} {}", temp_mode, temp_filename);
        let (chmod_temp_exit, _, chmod_temp_stderr) = self.execute_command(&chmod_temp_cmd)?;
        if chmod_temp_exit != 0 {
            warn!("Failed to set temporary file permissions: {}", chmod_temp_stderr);
        } else {
            debug!("Set temporary file mode to: {}", temp_mode);
        }
        
        // Step 3: Move the temporary file to the target location with sudo
        let move_cmd = format!("mv {} {}", temp_filename, remote_path);
        let (exit_code, _, stderr) = self.execute_sudo_command(&move_cmd, "")?;
        if exit_code != 0 {
            // Clean up temp file if move failed
            let _ = self.execute_command(&format!("rm -f {}", temp_filename));
            return Err(anyhow!("Failed to move file to target location: {}", stderr));
        }
        
        // Step 4: Ensure file permissions are correct after move
        // Determine the final mode: explicit mode > original mode > default 644
        let final_mode = mode
            .map(|m| m.to_string())
            .or(original_mode)
            .unwrap_or_else(|| "644".to_string());
            
        let chmod_cmd = format!("chmod {} {}", final_mode, remote_path);
        let (chmod_exit_code, _, chmod_stderr) = self.execute_sudo_command(&chmod_cmd, "")?;
        if chmod_exit_code != 0 {
            warn!("Failed to set file mode: {}", chmod_stderr);
        } else {
            debug!("Set final file mode to: {}", final_mode);
        }
        
        // Step 5: Set file ownership - use explicit params if provided, otherwise preserve original
        let target_owner = owner.map(|o| o.to_string()).or(original_owner);
        let target_group = group.map(|g| g.to_string()).or(original_group);
        
        if target_owner.is_some() || target_group.is_some() {
            let ownership = match (target_owner.as_deref(), target_group.as_deref()) {
                (Some(o), Some(g)) => format!("{}:{}", o, g),
                (Some(o), None) => o.to_string(),
                (None, Some(g)) => format!(":{}", g),
                (None, None) => String::new(),
            };
            
            if !ownership.is_empty() {
                let chown_cmd = format!("chown {} {}", ownership, remote_path);
                let (chown_exit_code, _, chown_stderr) = self.execute_sudo_command(&chown_cmd, "")?;
                if chown_exit_code != 0 {
                    warn!("Failed to set file ownership: {}", chown_stderr);
                } else {
                    debug!("Set file ownership to: {}", ownership);
                }
            }
        }
        
        info!("Successfully wrote file {} with sudo privileges", remote_path);
        Ok(())
    }
    
    /// Write content to a file without sudo (for regular file operations)
    pub fn write_file_content(&self, remote_path: &str, content: &str) -> Result<()> {
        debug!("Writing content to file: {}", remote_path);
        
        // Use echo with proper escaping for shell safety
        let escaped_content = content.replace('\'', "'\\''");
        let cmd = format!("echo '{}' > {}", escaped_content, remote_path);
        
        let (exit_code, _, stderr) = self.execute_command(&cmd)?;
        if exit_code != 0 {
            return Err(anyhow!("Failed to write file content: {}", stderr));
        }
        
        Ok(())
    }

    /// Upload a local file with sudo privileges (legacy method, kept for compatibility)
    #[allow(dead_code)]
    pub fn upload_sudo_file(&self, local_path: &str, remote_path: &str) -> Result<()> {
        debug!("Uploading file {} to {} with sudo", local_path, remote_path);
        
        // Read local file content
        let content = std::fs::read_to_string(local_path)
            .context(format!("Failed to read local file: {}", local_path))?;
        
        // Use the new write_file_with_sudo method
        self.write_file_with_sudo(&content, remote_path, None, None, None)
    }
        
    #[allow(dead_code)]
    pub fn upload_file(&self, local_path: &str, remote_path: &str) -> Result<()> {
        let local_content = std::fs::read(local_path)
            .context(format!("Failed to read local file: {}", local_path))?;
        
        let mut remote_file = self.session.scp_send(
                Path::new(remote_path), 
                0o644, 
                local_content.len() as u64, 
                None
            )
            .context(format!("Failed to initiate SCP upload to {}", remote_path))?;
        
        remote_file.write_all(&local_content)
            .context("Failed to write file contents via SCP")?;
        
        // Finish the upload
        remote_file.send_eof().context("Failed to finalize SCP upload")?;
        remote_file.wait_eof().context("Failed to wait for SCP EOF confirmation")?;
        remote_file.close().context("Failed to close SCP channel")?;
        remote_file.wait_close().context("Failed to wait for SCP channel close")?;
        
        Ok(())
    }
} 