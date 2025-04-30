use anyhow::{Result, Context, anyhow};
use ssh2::Session;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use log::{debug, info, warn};

use crate::inventory::Host;

pub struct SshClient {
    session: Session,
    host: String,
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
        })
    }
    
    pub fn execute_command(&self, command: &str) -> Result<(i32, String, String)> {
        debug!("Executing command on {}: {}", self.host, command);
        
        let mut channel = self.session.channel_session()
            .context("Failed to open SSH channel")?;
        
        channel.exec(command).context(format!("Failed to execute command: {}", command))?;
        
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
            format!("sudo -n {}", command)
        } else {
            format!("sudo -n -u {} {}", sudo_user, command)
        };
        
        debug!("Executing sudo command on {}: {}", self.host, sudo_cmd);
        self.execute_command(&sudo_cmd)
    }
    
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
    
    // 未使用的方法，保留以便将来可能需要
    /*
    pub fn download_file(&self, remote_path: &str, local_path: &str) -> Result<()> {
        let (mut remote_file, _) = self.session.scp_recv(Path::new(remote_path))
            .context(format!("Failed to initiate SCP download from {}", remote_path))?;
        
        let mut contents = Vec::new();
        remote_file.read_to_end(&mut contents)
            .context("Failed to read file contents via SCP")?;
        
        std::fs::write(local_path, contents)
            .context(format!("Failed to write downloaded content to {}", local_path))?;
        
        Ok(())
    }
    */
} 