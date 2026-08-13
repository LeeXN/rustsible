use anyhow::{anyhow, bail, Context, Result};
use log::{debug, info, warn};
#[cfg(test)]
use mockall::automock;
use ssh2::{CheckResult, HashType, KnownHostFileKind, OpenFlags, OpenType, Session, Stream};
use std::fs::File;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

use crate::inventory::Host;

const DEFAULT_SSH_TIMEOUT_SECS: u64 = 10;
const DEFAULT_COMMAND_TIMEOUT_SECS: u64 = 300;
const MAX_COMMAND_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const IO_POLL_INTERVAL: Duration = Duration::from_millis(2);

#[cfg(unix)]
fn set_pipe_nonblocking<T: std::os::fd::AsRawFd>(pipe: &T) -> io::Result<()> {
    let descriptor = pipe.as_raw_fd();
    // SAFETY: `descriptor` is borrowed from a live stdio pipe. F_GETFL does
    // not dereference user pointers, and F_SETFL only adds O_NONBLOCK while
    // preserving the descriptor's existing status flags.
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_pipe_nonblocking<T>(_pipe: &T) -> io::Result<()> {
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionKind {
    Local,
    Ssh,
}

/// Resolve the transport without silently overriding an explicit inventory
/// choice. Bare localhost aliases remain convenient and use the local backend,
/// while `ansible_connection=ssh` always means SSH (including for localhost).
pub fn connection_kind(host: &Host) -> Result<ConnectionKind> {
    if let Some(connection) = host
        .get_variable("ansible_connection")
        .or_else(|| host.get_variable("ansible_connection_type"))
    {
        return match connection.trim().to_ascii_lowercase().as_str() {
            "local" => Ok(ConnectionKind::Local),
            "ssh" | "smart" | "paramiko" => Ok(ConnectionKind::Ssh),
            other => bail!("Unsupported connection type: {other}"),
        };
    }

    if matches!(
        host.hostname.trim().to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "::1"
    ) {
        Ok(ConnectionKind::Local)
    } else {
        Ok(ConnectionKind::Ssh)
    }
}

/// Quote one value for use as a single POSIX shell word.
///
/// This is intentionally public within the crate so modules that must invoke a
/// remote shell can use the same escaping rule instead of growing ad-hoc
/// variants. An entire shell program must not be passed to this function.
pub(crate) fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }

    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn parse_inventory_bool(name: &str, value: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "yes" | "true" | "on" => Ok(true),
        "0" | "no" | "false" | "off" => Ok(false),
        _ => bail!(
            "Inventory variable {} must be one of true/false, yes/no, on/off, or 1/0",
            name
        ),
    }
}

fn host_key_checking_enabled(host: &Host) -> Result<bool> {
    for name in [
        "rustsible_host_key_checking",
        "ansible_host_key_checking",
        "ansible_ssh_host_key_checking",
    ] {
        if let Some(value) = host.get_variable(name) {
            return parse_inventory_bool(name, value);
        }
    }

    Ok(true)
}

fn ssh_timeout(host: &Host) -> Result<Duration> {
    let timeout_value = host
        .get_variable("ansible_ssh_timeout")
        .or_else(|| host.get_variable("ansible_timeout"));

    let seconds = match timeout_value {
        Some(value) => value
            .trim()
            .parse::<u64>()
            .context("SSH timeout must be a positive integer number of seconds")?,
        None => DEFAULT_SSH_TIMEOUT_SECS,
    };

    if seconds == 0 {
        bail!("SSH timeout must be greater than zero");
    }

    // libssh2 accepts a u32 millisecond timeout. Fail rather than silently
    // wrapping an inventory value into a much shorter timeout.
    let millis = seconds
        .checked_mul(1_000)
        .filter(|millis| *millis <= u32::MAX as u64)
        .context("SSH timeout is too large")?;

    Ok(Duration::from_millis(millis))
}

fn command_timeout(host: &Host) -> Result<Duration> {
    let timeout_value = host
        .get_variable("rustsible_command_timeout")
        .or_else(|| host.get_variable("ansible_command_timeout"));
    let seconds = match timeout_value {
        Some(value) => value
            .trim()
            .parse::<u64>()
            .context("SSH command timeout must be a positive integer number of seconds")?,
        None => DEFAULT_COMMAND_TIMEOUT_SECS,
    };
    if seconds == 0 {
        bail!("SSH command timeout must be greater than zero");
    }
    Ok(Duration::from_secs(seconds))
}

fn known_hosts_paths(host: &Host) -> Result<(Vec<PathBuf>, bool)> {
    if let Some(path) = host
        .get_variable("rustsible_known_hosts_file")
        .or_else(|| host.get_variable("ansible_ssh_known_hosts_file"))
    {
        if path.trim().is_empty() {
            bail!("The configured known_hosts file path cannot be empty");
        }
        return Ok((vec![PathBuf::from(path)], true));
    }

    let mut paths = vec![PathBuf::from("/etc/ssh/ssh_known_hosts")];
    if let Some(home) = std::env::var_os("HOME") {
        paths.push(PathBuf::from(home).join(".ssh").join("known_hosts"));
    }
    Ok((paths, false))
}

fn fingerprint(session: &Session) -> String {
    session
        .host_key_hash(HashType::Sha256)
        .map(|hash| {
            hash.iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .join(":")
        })
        .unwrap_or_else(|| "unavailable".to_string())
}

fn verify_host_key(session: &Session, host: &Host) -> Result<()> {
    if !host_key_checking_enabled(host)? {
        warn!(
            "SSH host key checking is explicitly disabled for {}; this connection is vulnerable to interception",
            host.name
        );
        return Ok(());
    }

    let (paths, explicitly_configured) = known_hosts_paths(host)?;
    let mut known_hosts = session
        .known_hosts()
        .context("Failed to initialize SSH known_hosts verification")?;
    let mut loaded_files = 0_u32;

    for path in paths {
        if !path.is_file() {
            if explicitly_configured {
                bail!("Configured SSH known_hosts file does not exist");
            }
            continue;
        }

        known_hosts
            .read_file(&path, KnownHostFileKind::OpenSSH)
            .context("Failed to read an SSH known_hosts file")?;
        loaded_files += 1;
    }

    debug!("Loaded {} SSH known_hosts file(s)", loaded_files);

    let (server_key, _) = session
        .host_key()
        .context("SSH server did not provide a host key")?;
    let result = known_hosts.check_port(&host.hostname, host.port, server_key);
    let key_fingerprint = fingerprint(session);

    match result {
        CheckResult::Match => {
            debug!("SSH host key verified for {}", host.name);
            Ok(())
        }
        CheckResult::Mismatch => bail!(
            "SSH host key mismatch for {} (SHA256 fingerprint bytes: {}); possible interception",
            host.name,
            key_fingerprint
        ),
        CheckResult::NotFound => bail!(
            "SSH host key for {} is not present in known_hosts (SHA256 fingerprint bytes: {})",
            host.name,
            key_fingerprint
        ),
        CheckResult::Failure => bail!("Failed to verify SSH host key for {}", host.name),
    }
}

fn connect_tcp(hostname: &str, port: u16, timeout: Duration) -> Result<TcpStream> {
    let addresses = (hostname, port)
        .to_socket_addrs()
        .with_context(|| format!("Failed to resolve {hostname}:{port}"))?;
    let mut last_error = None;

    for address in addresses {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => {
                stream
                    .set_read_timeout(Some(timeout))
                    .context("Failed to configure SSH socket read timeout")?;
                stream
                    .set_write_timeout(Some(timeout))
                    .context("Failed to configure SSH socket write timeout")?;
                stream
                    .set_nodelay(true)
                    .context("Failed to configure SSH TCP socket")?;
                return Ok(stream);
            }
            Err(error) => last_error = Some(error),
        }
    }

    match last_error {
        Some(error) => Err(error)
            .with_context(|| format!("Failed to connect to {hostname}:{port} within {timeout:?}")),
        None => bail!("No network addresses were found for {hostname}:{port}"),
    }
}

#[cfg(unix)]
fn spawn_local_shell(command: &str) -> io::Result<Child> {
    use std::os::unix::process::CommandExt;

    let mut process = Command::new("sh");
    process
        .args(["-c", command])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    process.spawn()
}

#[cfg(windows)]
fn spawn_local_shell(command: &str) -> io::Result<Child> {
    Command::new("cmd")
        .args(["/C", command])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

#[cfg(not(any(unix, windows)))]
fn spawn_local_shell(command: &str) -> io::Result<Child> {
    Command::new("sh")
        .args(["-c", command])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

#[cfg(unix)]
fn terminate_local_process(child: &mut Child) -> Result<()> {
    terminate_local_process_group(child.id());
    if child
        .try_wait()
        .context("Failed to query local process after termination")?
        .is_none()
    {
        child
            .kill()
            .context("Failed to terminate timed-out local process group")?;
    }
    let _ = child.wait();
    Ok(())
}

#[cfg(unix)]
fn terminate_local_process_group(process_id: u32) {
    let process_group = format!("-{process_id}");
    let group_kill = Command::new("kill")
        .args(["-KILL", "--", &process_group])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    // The group may already be empty. This cleanup is best effort; callers
    // separately terminate and reap the direct child when it is still alive.
    let _ = group_kill;
}

#[cfg(not(unix))]
fn terminate_local_process(child: &mut Child) -> Result<()> {
    // std has no portable process-tree termination API. On Windows this
    // terminates the direct command process; descendants may outlive it.
    child
        .kill()
        .context("Failed to terminate timed-out local command")?;
    let _ = child.wait();
    Ok(())
}

fn append_bounded(
    target: &mut Vec<u8>,
    bytes: &[u8],
    combined_output_size: &mut usize,
) -> Result<()> {
    let new_size = combined_output_size
        .checked_add(bytes.len())
        .context("SSH command output size overflow")?;
    if new_size > MAX_COMMAND_OUTPUT_BYTES {
        bail!(
            "SSH command output exceeded the {} byte safety limit",
            MAX_COMMAND_OUTPUT_BYTES
        );
    }
    target.extend_from_slice(bytes);
    *combined_output_size = new_size;
    Ok(())
}

fn read_stream_once(
    stream: &mut Stream,
    output: &mut Vec<u8>,
    done: &mut bool,
    combined_output_size: &mut usize,
) -> Result<bool> {
    if *done {
        return Ok(false);
    }

    let mut buffer = [0_u8; 16 * 1024];
    match stream.read(&mut buffer) {
        Ok(0) => {
            *done = true;
            Ok(true)
        }
        Ok(read) => {
            append_bounded(output, &buffer[..read], combined_output_size)?;
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(false),
        Err(error) => Err(error).context("Failed to read SSH command output"),
    }
}

fn read_bounded_local_output<R: Read>(
    mut reader: R,
    combined_size: Arc<AtomicUsize>,
    exceeded: Arc<AtomicBool>,
    completed: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        if cancelled.load(Ordering::Acquire) {
            completed.store(true, Ordering::Release);
            return Ok(output);
        }
        let read = match reader.read(&mut buffer) {
            Ok(0) => {
                completed.store(true, Ordering::Release);
                return Ok(output);
            }
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(IO_POLL_INTERVAL);
                continue;
            }
            Err(error) => {
                completed.store(true, Ordering::Release);
                return Err(error);
            }
        };
        let previous = combined_size.fetch_add(read, Ordering::AcqRel);
        if previous
            .checked_add(read)
            .map(|size| size > MAX_COMMAND_OUTPUT_BYTES)
            .unwrap_or(true)
        {
            exceeded.store(true, Ordering::Release);
            completed.store(true, Ordering::Release);
            return Err(io::Error::other("local command output limit exceeded"));
        }
        output.extend_from_slice(&buffer[..read]);
    }
}

fn write_local_input_nonblocking<W: Write>(
    mut writer: W,
    input: Vec<u8>,
    completed: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
) -> io::Result<()> {
    let mut offset = 0usize;
    while offset < input.len() {
        if cancelled.load(Ordering::Acquire) {
            completed.store(true, Ordering::Release);
            return Ok(());
        }
        match writer.write(&input[offset..]) {
            Ok(0) => std::thread::sleep(IO_POLL_INTERVAL),
            Ok(written) => offset += written,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(IO_POLL_INTERVAL);
            }
            Err(error) => {
                completed.store(true, Ordering::Release);
                return Err(error);
            }
        }
    }
    loop {
        if cancelled.load(Ordering::Acquire) {
            completed.store(true, Ordering::Release);
            return Ok(());
        }
        match writer.flush() {
            Ok(()) => {
                completed.store(true, Ordering::Release);
                return Ok(());
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(IO_POLL_INTERVAL);
            }
            Err(error) => {
                completed.store(true, Ordering::Release);
                return Err(error);
            }
        }
    }
}

struct BlockingModeGuard<'a> {
    session: &'a Session,
    previous: bool,
}

impl<'a> BlockingModeGuard<'a> {
    fn nonblocking(session: &'a Session) -> Self {
        let previous = session.is_blocking();
        session.set_blocking(false);
        Self { session, previous }
    }
}

impl Drop for BlockingModeGuard<'_> {
    fn drop(&mut self) {
        self.session.set_blocking(self.previous);
    }
}

fn build_noninteractive_sudo_command(command: &str, sudo_user: &str) -> String {
    let user_option = if sudo_user.is_empty() || sudo_user == "root" {
        String::new()
    } else {
        format!(" -u {}", shell_quote(sudo_user))
    };

    format!(
        "sudo -n -p ''{} -- sh -c {}",
        user_option,
        shell_quote(command)
    )
}

fn build_sudo_validation_command(sudo_user: &str) -> String {
    let user_option = if sudo_user.is_empty() || sudo_user == "root" {
        String::new()
    } else {
        format!(" -u {}", shell_quote(sudo_user))
    };
    format!("sudo -S -p ''{} -v", user_option)
}

fn sudo_password_input(password: &str) -> Result<Option<Vec<u8>>> {
    if password.contains(['\n', '\r', '\0']) {
        bail!("The sudo password contains characters unsupported by sudo -S");
    }
    if password.is_empty() {
        Ok(None)
    } else {
        let mut input = Vec::with_capacity(password.len() + 1);
        input.extend_from_slice(password.as_bytes());
        input.push(b'\n');
        Ok(Some(input))
    }
}

/// POSIX `test` has no portable `--` option terminator. After unary `-e`, the
/// next shell-quoted token is unambiguously the path, including names starting
/// with a dash.
fn test_path_exists_command(path: &str) -> String {
    format!("test -e {}", shell_quote(path))
}

/// Trait defining SSH connection operations for mocking.
#[cfg_attr(test, automock)]
pub trait SshConnection {
    fn is_local(&self) -> bool {
        false
    }
    fn execute_command(&self, command: &str) -> Result<(i32, String, String)>;
    fn execute_command_with_input(
        &self,
        command: &str,
        input: &[u8],
    ) -> Result<(i32, String, String)>;
    fn execute_sudo_command(&self, command: &str, sudo_user: &str)
        -> Result<(i32, String, String)>;
    fn execute_sudo_command_with_input(
        &self,
        command: &str,
        sudo_user: &str,
        input: &[u8],
    ) -> Result<(i32, String, String)>;
    fn write_file_with_sudo(
        &self,
        content: &str,
        remote_path: &str,
        sudo_user: &str,
        mode: Option<String>,
        owner: Option<String>,
        group: Option<String>,
    ) -> Result<()>;
    fn write_file_content(&self, remote_path: &str, content: &str) -> Result<()>;
    fn write_file_bytes(&self, remote_path: &str, content: &[u8]) -> Result<()>;
    fn write_file_bytes_with_sudo(
        &self,
        content: &[u8],
        remote_path: &str,
        sudo_user: &str,
        mode: Option<String>,
        owner: Option<String>,
        group: Option<String>,
    ) -> Result<()>;
    /// Read a regular file without text decoding. A missing path returns None.
    fn read_file_bytes(&self, path: &str) -> Result<Option<Vec<u8>>>;
    /// Read a regular file through the selected become identity without
    /// decoding it as text. A missing path returns None.
    fn read_file_bytes_with_sudo(&self, path: &str, sudo_user: &str) -> Result<Option<Vec<u8>>> {
        let inspect = test_path_exists_command(path);
        let (exists, _, stderr) = self.execute_sudo_command(&inspect, sudo_user)?;
        match exists {
            0 => {}
            1 => return Ok(None),
            code => bail!(
                "Failed to inspect privileged file (exit {}): {}",
                code,
                stderr.trim()
            ),
        }
        let command = format!("cat -- {}", shell_quote(path));
        let (code, stdout, stderr) = self.execute_sudo_command(&command, sudo_user)?;
        if code != 0 {
            bail!(
                "Failed to read privileged file (exit {}): {}",
                code,
                stderr.trim()
            );
        }
        Ok(Some(stdout.into_bytes()))
    }
    fn upload_file(&self, local_path: &str, remote_path: &str) -> Result<()>;
}

/// A connection selected for one inventory host.
pub enum Connection {
    Local(LocalConnection),
    Ssh(SshClient),
}

impl Connection {
    pub fn connect(host: &Host) -> Result<Self> {
        match connection_kind(host)? {
            ConnectionKind::Local => Ok(Self::Local(LocalConnection::new(host)?)),
            ConnectionKind::Ssh => Ok(Self::Ssh(SshClient::connect(host)?)),
        }
    }

    pub fn as_connection(&self) -> &dyn SshConnection {
        match self {
            Self::Local(connection) => connection,
            Self::Ssh(connection) => connection,
        }
    }
}

pub struct LocalConnection {
    host: String,
    sudo_password: String,
    command_timeout: Duration,
}

impl LocalConnection {
    pub fn new(host: &Host) -> Result<Self> {
        if connection_kind(host)? != ConnectionKind::Local {
            bail!("Host '{}' is not configured for local execution", host.name);
        }
        Ok(Self {
            host: host.name.clone(),
            sudo_password: host
                .get_ssh_sudo_password()
                .map(String::as_str)
                .unwrap_or("")
                .to_string(),
            command_timeout: command_timeout(host)?,
        })
    }

    fn execute_with_input_bytes(
        &self,
        command: &str,
        input: Option<&[u8]>,
    ) -> Result<(i32, Vec<u8>, Vec<u8>)> {
        debug!(
            "Executing local command for {} ({} bytes)",
            self.host,
            command.len()
        );

        // Start the deadline before spawning the process. In particular, a
        // large stdin must not get an unbounded grace period while the child
        // is itself blocked writing stdout or stderr.
        let deadline = Instant::now()
            .checked_add(self.command_timeout)
            .context("Local command timeout is too large")?;
        let mut child = spawn_local_shell(command).context("Failed to start local command")?;

        // Drain both output pipes while stdin is being written. Writing stdin
        // synchronously before starting the readers deadlocks when the child
        // fills an output pipe before it starts reading its input.
        let stdin = child
            .stdin
            .take()
            .context("Local command stdin is unavailable")?;
        set_pipe_nonblocking(&stdin).context("Failed to configure local stdin pipe")?;
        let input_completed = Arc::new(AtomicBool::new(input.is_none()));
        let input_cancelled = Arc::new(AtomicBool::new(false));
        let mut stdin_writer = match input {
            Some(input) => {
                let input = input.to_vec();
                let writer_completed = Arc::clone(&input_completed);
                let writer_cancelled = Arc::clone(&input_cancelled);
                Some(std::thread::spawn(move || {
                    write_local_input_nonblocking(stdin, input, writer_completed, writer_cancelled)
                }))
            }
            None => {
                drop(stdin);
                None
            }
        };

        let mut stdout = child
            .stdout
            .take()
            .context("Local command stdout is unavailable")?;
        let mut stderr = child
            .stderr
            .take()
            .context("Local command stderr is unavailable")?;
        set_pipe_nonblocking(&stdout).context("Failed to configure local stdout pipe")?;
        set_pipe_nonblocking(&stderr).context("Failed to configure local stderr pipe")?;
        let combined_size = Arc::new(AtomicUsize::new(0));
        let output_exceeded = Arc::new(AtomicBool::new(false));
        let output_cancelled = Arc::new(AtomicBool::new(false));
        let stdout_size = Arc::clone(&combined_size);
        let stdout_exceeded = Arc::clone(&output_exceeded);
        let stdout_cancelled = Arc::clone(&output_cancelled);
        let stdout_completed = Arc::new(AtomicBool::new(false));
        let stdout_completed_reader = Arc::clone(&stdout_completed);
        let stdout_reader = std::thread::spawn(move || {
            read_bounded_local_output(
                &mut stdout,
                stdout_size,
                stdout_exceeded,
                stdout_completed_reader,
                stdout_cancelled,
            )
        });
        let stderr_size = Arc::clone(&combined_size);
        let stderr_exceeded = Arc::clone(&output_exceeded);
        let stderr_cancelled = Arc::clone(&output_cancelled);
        let stderr_completed = Arc::new(AtomicBool::new(false));
        let stderr_completed_reader = Arc::clone(&stderr_completed);
        let stderr_reader = std::thread::spawn(move || {
            read_bounded_local_output(
                &mut stderr,
                stderr_size,
                stderr_exceeded,
                stderr_completed_reader,
                stderr_cancelled,
            )
        });

        let status = loop {
            if output_exceeded.load(Ordering::Acquire) {
                output_cancelled.store(true, Ordering::Release);
                input_cancelled.store(true, Ordering::Release);
                terminate_local_process(&mut child)?;
                if let Some(writer) = stdin_writer.take() {
                    let _ = writer.join();
                }
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                bail!(
                    "Local command output exceeded the {} byte safety limit",
                    MAX_COMMAND_OUTPUT_BYTES
                );
            }
            if let Some(status) = child
                .try_wait()
                .context("Failed to query local command status")?
            {
                // A shell may exit after launching a background descendant
                // that inherited stdout/stderr. Kill the isolated process
                // group before joining readers so EOF remains bounded by the
                // command deadline instead of hanging forever.
                #[cfg(unix)]
                terminate_local_process_group(child.id());
                break status;
            }
            if Instant::now() >= deadline {
                output_cancelled.store(true, Ordering::Release);
                input_cancelled.store(true, Ordering::Release);
                terminate_local_process(&mut child)?;
                // The killed process closes its pipe handles; join readers so
                // no detached thread survives this operation.
                if let Some(writer) = stdin_writer.take() {
                    let _ = writer.join();
                }
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                bail!("Local command timed out after {:?}", self.command_timeout);
            }
            std::thread::sleep(IO_POLL_INTERVAL);
        };

        // A descendant can create a new session/process group and keep the
        // inherited pipes open after the direct child exits. Do not let such a
        // detached process bypass the same command deadline. Closing our read
        // ends unblocks both reader threads; they then finish without being
        // joined indefinitely.
        while !stdout_completed.load(Ordering::Acquire)
            || !stderr_completed.load(Ordering::Acquire)
            || !input_completed.load(Ordering::Acquire)
        {
            if Instant::now() >= deadline {
                output_cancelled.store(true, Ordering::Release);
                input_cancelled.store(true, Ordering::Release);
                if let Some(writer) = stdin_writer.take() {
                    let _ = writer.join();
                }
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                bail!("Local command timed out after {:?}", self.command_timeout);
            }
            if output_exceeded.load(Ordering::Acquire) {
                output_cancelled.store(true, Ordering::Release);
                input_cancelled.store(true, Ordering::Release);
                if let Some(writer) = stdin_writer.take() {
                    let _ = writer.join();
                }
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                bail!(
                    "Local command output exceeded the {} byte safety limit",
                    MAX_COMMAND_OUTPUT_BYTES
                );
            }
            std::thread::sleep(IO_POLL_INTERVAL);
        }
        if let Some(writer) = stdin_writer.take() {
            writer
                .join()
                .map_err(|_| anyhow!("Local stdin writer thread panicked"))?
                .context("Failed to write local command input")?;
        }
        let stdout = stdout_reader
            .join()
            .map_err(|_| anyhow!("Local stdout reader thread panicked"))?
            .context("Failed to read local command stdout")?;
        let stderr = stderr_reader
            .join()
            .map_err(|_| anyhow!("Local stderr reader thread panicked"))?
            .context("Failed to read local command stderr")?;
        let combined_size = stdout
            .len()
            .checked_add(stderr.len())
            .context("Local command output size overflow")?;
        if combined_size > MAX_COMMAND_OUTPUT_BYTES {
            bail!(
                "Local command output exceeded the {} byte safety limit",
                MAX_COMMAND_OUTPUT_BYTES
            );
        }

        let exit_code = status.code().unwrap_or(1);
        debug!("Local command completed with exit code: {}", exit_code);
        Ok((exit_code, stdout, stderr))
    }

    fn execute_with_input(
        &self,
        command: &str,
        input: Option<&[u8]>,
    ) -> Result<(i32, String, String)> {
        let (code, stdout, stderr) = self.execute_with_input_bytes(command, input)?;
        Ok((
            code,
            String::from_utf8_lossy(&stdout).into_owned(),
            String::from_utf8_lossy(&stderr).into_owned(),
        ))
    }

    fn apply_local_metadata(
        &self,
        path: &str,
        sudo_user: &str,
        mode: Option<String>,
        owner: Option<String>,
        group: Option<String>,
    ) -> Result<()> {
        if let Some(mode) = mode {
            let command = format!("chmod -- {} {}", shell_quote(&mode), shell_quote(path));
            let (code, _, stderr) = self.execute_sudo_command(&command, sudo_user)?;
            if code != 0 {
                bail!("Failed to set local file permissions: {}", stderr.trim());
            }
        }
        if owner.is_some() || group.is_some() {
            let ownership = match (owner.as_deref(), group.as_deref()) {
                (Some(owner), Some(group)) => format!("{owner}:{group}"),
                (Some(owner), None) => owner.to_string(),
                (None, Some(group)) => format!(":{group}"),
                (None, None) => return Ok(()),
            };
            let command = format!("chown -- {} {}", shell_quote(&ownership), shell_quote(path));
            let (code, _, stderr) = self.execute_sudo_command(&command, sudo_user)?;
            if code != 0 {
                bail!("Failed to set local file ownership: {}", stderr.trim());
            }
        }
        Ok(())
    }
}

impl SshConnection for LocalConnection {
    fn is_local(&self) -> bool {
        true
    }

    fn execute_command(&self, command: &str) -> Result<(i32, String, String)> {
        self.execute_with_input(command, None)
    }

    fn execute_command_with_input(
        &self,
        command: &str,
        input: &[u8],
    ) -> Result<(i32, String, String)> {
        self.execute_with_input(command, Some(input))
    }

    fn execute_sudo_command(
        &self,
        command: &str,
        sudo_user: &str,
    ) -> Result<(i32, String, String)> {
        self.execute_sudo_command_with_input(command, sudo_user, &[])
    }

    fn execute_sudo_command_with_input(
        &self,
        command: &str,
        sudo_user: &str,
        input: &[u8],
    ) -> Result<(i32, String, String)> {
        if let Some(password_input) = sudo_password_input(&self.sudo_password)? {
            let validation = build_sudo_validation_command(sudo_user);
            let (code, _, stderr) = self.execute_with_input(&validation, Some(&password_input))?;
            if code != 0 {
                bail!("Local sudo authentication failed: {}", stderr.trim());
            }
        }
        let sudo_command = build_noninteractive_sudo_command(command, sudo_user);
        self.execute_with_input(&sudo_command, Some(input))
    }

    fn write_file_with_sudo(
        &self,
        content: &str,
        remote_path: &str,
        sudo_user: &str,
        mode: Option<String>,
        owner: Option<String>,
        group: Option<String>,
    ) -> Result<()> {
        self.write_file_bytes_with_sudo(
            content.as_bytes(),
            remote_path,
            sudo_user,
            mode,
            owner,
            group,
        )
    }

    fn write_file_bytes_with_sudo(
        &self,
        content: &[u8],
        remote_path: &str,
        sudo_user: &str,
        mode: Option<String>,
        owner: Option<String>,
        group: Option<String>,
    ) -> Result<()> {
        // Feed bytes directly to a file opened by the selected become user.
        // A controller-owned 0600 temporary file is not readable when
        // `become_user` is a non-root account.
        let install_command = format!("umask 077; tee -- {} >/dev/null", shell_quote(remote_path));
        let (code, _, stderr) =
            self.execute_sudo_command_with_input(&install_command, sudo_user, content)?;
        if code != 0 {
            bail!("Failed to install local privileged file: {}", stderr.trim());
        }
        self.apply_local_metadata(remote_path, sudo_user, mode, owner, group)
    }

    fn write_file_content(&self, remote_path: &str, content: &str) -> Result<()> {
        self.write_file_bytes(remote_path, content.as_bytes())
    }

    fn write_file_bytes(&self, remote_path: &str, content: &[u8]) -> Result<()> {
        std::fs::write(remote_path, content)
            .with_context(|| format!("Failed to write local file: {remote_path}"))
    }

    fn read_file_bytes(&self, path: &str) -> Result<Option<Vec<u8>>> {
        match std::fs::read(path) {
            Ok(content) => Ok(Some(content)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("Failed to read local file: {path}")),
        }
    }

    fn read_file_bytes_with_sudo(&self, path: &str, sudo_user: &str) -> Result<Option<Vec<u8>>> {
        let inspect = test_path_exists_command(path);
        let (exists, _, stderr) = self.execute_sudo_command(&inspect, sudo_user)?;
        match exists {
            0 => {}
            1 => return Ok(None),
            code => bail!(
                "Failed to inspect privileged local file (exit {}): {}",
                code,
                stderr.trim()
            ),
        }
        let command = format!("cat -- {}", shell_quote(path));
        if let Some(password_input) = sudo_password_input(&self.sudo_password)? {
            let validation = build_sudo_validation_command(sudo_user);
            let (code, _, stderr) = self.execute_with_input(&validation, Some(&password_input))?;
            if code != 0 {
                bail!("Local sudo authentication failed: {}", stderr.trim());
            }
        }
        let sudo_command = build_noninteractive_sudo_command(&command, sudo_user);
        let (code, stdout, stderr) = self.execute_with_input_bytes(&sudo_command, None)?;
        if code != 0 {
            bail!(
                "Failed to read privileged local file (exit {}): {}",
                code,
                String::from_utf8_lossy(&stderr).trim()
            );
        }
        Ok(Some(stdout))
    }

    fn upload_file(&self, local_path: &str, remote_path: &str) -> Result<()> {
        std::fs::copy(local_path, remote_path)
            .with_context(|| format!("Failed to copy local file to {remote_path}"))?;
        Ok(())
    }
}

pub struct SshClient {
    session: Session,
    host: String,
    sudo_password: String,
    command_timeout: Duration,
    operation_lock: Mutex<()>,
}

impl SshClient {
    fn execute_with_input_bytes(
        &self,
        command: &str,
        input: Option<&[u8]>,
    ) -> Result<(i32, Vec<u8>, Vec<u8>)> {
        let _operation = self
            .operation_lock
            .lock()
            .map_err(|_| anyhow!("SSH operation lock was poisoned"))?;

        // Commands can contain module arguments and credentials. Log only
        // metadata; never copy the command text into logs or error contexts.
        debug!(
            "Executing SSH command on {} ({} bytes)",
            self.host,
            command.len()
        );

        let deadline = Instant::now()
            .checked_add(self.command_timeout)
            .context("SSH command timeout is too large")?;
        let mode_guard = BlockingModeGuard::nonblocking(&self.session);

        let mut channel = loop {
            match self.session.channel_session() {
                Ok(channel) => break channel,
                Err(error) => {
                    let error: io::Error = error.into();
                    if error.kind() != io::ErrorKind::WouldBlock {
                        return Err(error).context("Failed to open SSH channel");
                    }
                }
            }
            if Instant::now() >= deadline {
                bail!("SSH command timed out after {:?}", self.command_timeout);
            }
            std::thread::sleep(IO_POLL_INTERVAL);
        };

        // Starting the remote process is also part of the command deadline.
        // libssh2 operations in non-blocking mode must be retried when their
        // SSH window or socket would block.
        loop {
            match channel.exec(command) {
                Ok(()) => break,
                Err(error) => {
                    let error: io::Error = error.into();
                    if error.kind() != io::ErrorKind::WouldBlock {
                        return Err(error).context("Failed to start remote SSH command");
                    }
                }
            }
            if Instant::now() >= deadline {
                bail!("SSH command timed out after {:?}", self.command_timeout);
            }
            std::thread::sleep(IO_POLL_INTERVAL);
        }

        let mut stdout_stream = channel.stream(0);
        let mut stderr_stream = channel.stderr();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut stdout_done = false;
        let mut stderr_done = false;
        let mut combined_output_size = 0;
        let input = input.unwrap_or_default();
        let mut input_offset = 0;
        let mut input_flushed = false;
        let mut input_eof_sent = false;

        // Progress stdin, stdout, and stderr together. A remote program is
        // allowed to emit more than an SSH window before reading stdin, so
        // completing either direction first can deadlock the other one.
        while !input_eof_sent || !stdout_done || !stderr_done {
            let mut input_progress = false;
            if input_offset < input.len() {
                match channel.write(&input[input_offset..]) {
                    Ok(0) => {}
                    Ok(written) => {
                        input_offset += written;
                        input_progress = true;
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) => return Err(error).context("Failed to write SSH command input"),
                }
            } else if !input_flushed {
                match channel.flush() {
                    Ok(()) => {
                        input_flushed = true;
                        input_progress = true;
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) => return Err(error).context("Failed to flush SSH command input"),
                }
            } else if !input_eof_sent {
                match channel.send_eof() {
                    Ok(()) => {
                        input_eof_sent = true;
                        input_progress = true;
                    }
                    Err(error) => {
                        let error: io::Error = error.into();
                        if error.kind() != io::ErrorKind::WouldBlock {
                            return Err(error).context("Failed to close SSH command input");
                        }
                    }
                }
            }

            let stdout_progress = read_stream_once(
                &mut stdout_stream,
                &mut stdout,
                &mut stdout_done,
                &mut combined_output_size,
            )?;
            let stderr_progress = read_stream_once(
                &mut stderr_stream,
                &mut stderr,
                &mut stderr_done,
                &mut combined_output_size,
            )?;

            if Instant::now() >= deadline {
                bail!("SSH command timed out after {:?}", self.command_timeout);
            }
            if !input_progress && !stdout_progress && !stderr_progress {
                std::thread::sleep(IO_POLL_INTERVAL);
            }
        }

        // Keep wait_close non-blocking as well so a server that sends stream
        // EOF but never closes the channel cannot escape the deadline.
        loop {
            match channel.wait_close() {
                Ok(()) => break,
                Err(error) => {
                    let error: io::Error = error.into();
                    if error.kind() != io::ErrorKind::WouldBlock {
                        return Err(error).context("Failed to wait for SSH command completion");
                    }
                }
            }
            if Instant::now() >= deadline {
                bail!("SSH command timed out after {:?}", self.command_timeout);
            }
            std::thread::sleep(IO_POLL_INTERVAL);
        }
        let exit_status = channel
            .exit_status()
            .context("Failed to get SSH command exit status")?;
        drop(mode_guard);

        debug!("SSH command completed with exit code: {}", exit_status);
        Ok((exit_status, stdout, stderr))
    }

    fn execute_with_input(
        &self,
        command: &str,
        input: Option<&[u8]>,
    ) -> Result<(i32, String, String)> {
        let (code, stdout, stderr) = self.execute_with_input_bytes(command, input)?;
        Ok((
            code,
            String::from_utf8_lossy(&stdout).into_owned(),
            String::from_utf8_lossy(&stderr).into_owned(),
        ))
    }

    fn write_remote_bytes(
        &self,
        remote_path: &str,
        content: &[u8],
        mode: i32,
        exclusive: bool,
    ) -> Result<()> {
        let _operation = self
            .operation_lock
            .lock()
            .map_err(|_| anyhow!("SSH operation lock was poisoned"))?;
        let sftp = self
            .session
            .sftp()
            .context("Failed to initialize SFTP subsystem")?;
        let flags = if exclusive {
            OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUSIVE
        } else {
            OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE
        };
        let mut remote_file = sftp
            .open_mode(Path::new(remote_path), flags, mode, OpenType::File)
            .context("Failed to open remote file through SFTP")?;

        remote_file
            .write_all(content)
            .context("Failed to write remote file through SFTP")?;
        remote_file
            .flush()
            .context("Failed to flush remote SFTP file")?;
        remote_file
            .close()
            .context("Failed to close remote SFTP file")?;
        Ok(())
    }

    fn cleanup_remote_temp(&self, remote_path: &str, sudo_user: &str) {
        let cleanup = format!("rm -f -- {}", shell_quote(remote_path));
        match self.execute_sudo_command(&cleanup, sudo_user) {
            Ok((0, _, _)) => {}
            Ok((exit_code, _, stderr)) => warn!(
                "Failed to clean up remote temporary file (exit {}): {}",
                exit_code,
                stderr.trim()
            ),
            Err(error) => warn!("Failed to clean up remote temporary file: {error:#}"),
        }
    }

    fn write_file_with_sudo_bytes(
        &self,
        content: &[u8],
        remote_path: &str,
        sudo_user: &str,
        mode: Option<String>,
        owner: Option<String>,
        group: Option<String>,
    ) -> Result<()> {
        let temp_filename = format!("/tmp/rustsible_temp_{}", Uuid::new_v4().simple());
        debug!("Writing privileged remote file through a unique temporary file");

        let result = (|| -> Result<()> {
            let quoted_target = shell_quote(remote_path);
            let check_cmd = format!("test -f {quoted_target}");
            let (file_exists_code, _, check_stderr) =
                self.execute_sudo_command(&check_cmd, sudo_user)?;
            let file_exists = match file_exists_code {
                0 => true,
                1 => false,
                code => bail!(
                    "Failed to inspect target file (exit {}): {}",
                    code,
                    check_stderr.trim()
                ),
            };

            let (original_mode, original_owner, original_group) = if file_exists {
                let stat_cmd = format!("stat -c '%a %U %G' -- {quoted_target}");
                let (stat_code, stat_output, stat_stderr) =
                    self.execute_sudo_command(&stat_cmd, sudo_user)?;
                if stat_code != 0 {
                    bail!("Failed to inspect target file: {}", stat_stderr.trim());
                }

                let parts: Vec<&str> = stat_output.split_whitespace().collect();
                if parts.len() != 3 {
                    bail!("Failed to parse target file metadata");
                }
                (
                    Some(parts[0].to_string()),
                    Some(parts[1].to_string()),
                    Some(parts[2].to_string()),
                )
            } else {
                (None, None, None)
            };

            // Create and write as the requested become identity. This keeps
            // arbitrary bytes intact and avoids a login-user-owned 0600 file
            // that a non-root become user cannot read.
            let create_command = format!("umask 077; set -C; : > {}", shell_quote(&temp_filename));
            let (create_code, _, create_stderr) =
                self.execute_sudo_command(&create_command, sudo_user)?;
            if create_code != 0 {
                bail!(
                    "Failed to create privileged temporary file: {}",
                    create_stderr.trim()
                );
            }
            let write_command = format!("tee -- {} >/dev/null", shell_quote(&temp_filename));
            let (write_code, _, write_stderr) =
                self.execute_sudo_command_with_input(&write_command, sudo_user, content)?;
            if write_code != 0 {
                bail!(
                    "Failed to write privileged temporary file: {}",
                    write_stderr.trim()
                );
            }

            let target_mode = mode.or(original_mode).unwrap_or_else(|| "644".to_string());
            let chmod_cmd = format!(
                "chmod -- {} {}",
                shell_quote(&target_mode),
                shell_quote(&temp_filename)
            );
            let (chmod_code, _, chmod_stderr) = self.execute_sudo_command(&chmod_cmd, sudo_user)?;
            if chmod_code != 0 {
                bail!(
                    "Failed to set temporary file permissions: {}",
                    chmod_stderr.trim()
                );
            }

            let target_owner = owner.or(original_owner);
            let target_group = group.or(original_group);
            let ownership = if let Some(owner) = target_owner.as_deref() {
                Some(match target_group.as_deref() {
                    Some(group) => format!("{owner}:{group}"),
                    None => owner.to_string(),
                })
            } else {
                target_group.as_deref().map(|group| format!(":{group}"))
            };
            if let Some(ownership) = ownership {
                let chown_cmd = format!(
                    "chown -- {} {}",
                    shell_quote(&ownership),
                    shell_quote(&temp_filename)
                );
                let (chown_code, _, chown_stderr) =
                    self.execute_sudo_command(&chown_cmd, sudo_user)?;
                if chown_code != 0 {
                    bail!(
                        "Failed to set temporary file ownership: {}",
                        chown_stderr.trim()
                    );
                }
            }

            let move_cmd = format!(
                "mv -- {} {}",
                shell_quote(&temp_filename),
                shell_quote(remote_path)
            );
            let (move_code, _, move_stderr) = self.execute_sudo_command(&move_cmd, sudo_user)?;
            if move_code != 0 {
                bail!(
                    "Failed to move file to target location: {}",
                    move_stderr.trim()
                );
            }

            Ok(())
        })();

        if result.is_err() {
            self.cleanup_remote_temp(&temp_filename, sudo_user);
        }
        result?;

        info!("Successfully wrote privileged remote file");
        Ok(())
    }

    pub fn connect(host: &Host) -> Result<Self> {
        info!(
            "Connecting to host: {} ({}:{})",
            host.name, host.hostname, host.port
        );
        debug!(
            "Host has {} direct and {} inherited inventory variable(s); values are not logged",
            host.variables.len(),
            host.inherited_variables.len()
        );

        if let Some(user) = host.get_ssh_user() {
            debug!("Using SSH user: {}", user);
        } else {
            debug!("No SSH user specified, using default: root");
        }
        debug!(
            "Password authentication is {}",
            if host.get_ssh_password().is_some() {
                "available"
            } else {
                "not configured"
            }
        );
        debug!(
            "Private-key authentication is {}",
            if host.get_ssh_private_key().is_some() {
                "available"
            } else {
                "not configured"
            }
        );

        let timeout = ssh_timeout(host)?;
        let command_timeout = command_timeout(host)?;
        let tcp = connect_tcp(&host.hostname, host.port, timeout)?;

        let mut session = Session::new().context("Failed to create SSH session")?;
        session.set_timeout(timeout.as_millis() as u32);
        session.set_tcp_stream(tcp);
        debug!("Starting SSH handshake with {}", host.hostname);
        session.handshake().context("SSH handshake failed")?;

        // Authenticate only after the server identity has been established.
        verify_host_key(&session, host)?;

        let username = host.get_ssh_user().map(String::as_str).unwrap_or("root");
        let sudo_password = host
            .get_ssh_sudo_password()
            .map(String::as_str)
            .unwrap_or("");

        debug!("Using SSH username: {}", username);
        let mut auth_succeeded = false;

        if let Some(password) = host.get_ssh_password() {
            debug!("Attempting password authentication for user {}", username);
            match session.userauth_password(username, password) {
                Ok(()) => {
                    info!("Password authentication succeeded for {}", username);
                    auth_succeeded = true;
                }
                Err(error) => warn!("Password authentication failed: {}", error),
            }
        }

        if !auth_succeeded {
            if let Some(key_path) = host.get_ssh_private_key() {
                debug!("Attempting private-key authentication");
                match session.userauth_pubkey_file(username, None, Path::new(key_path), None) {
                    Ok(()) => {
                        info!("Private-key authentication succeeded for {}", username);
                        auth_succeeded = true;
                    }
                    Err(error) => warn!("Private-key authentication failed: {}", error),
                }
            }
        }

        if !auth_succeeded {
            debug!("Attempting SSH agent authentication for user {}", username);
            match session.userauth_agent(username) {
                Ok(()) => {
                    info!("SSH agent authentication succeeded for {}", username);
                    auth_succeeded = true;
                }
                Err(error) => warn!("SSH agent authentication failed: {}", error),
            }
        }

        if !auth_succeeded {
            bail!(
                "All authentication methods failed for {}@{}",
                username,
                host.hostname
            );
        }

        Ok(Self {
            session,
            host: host.name.clone(),
            sudo_password: sudo_password.to_string(),
            command_timeout,
            operation_lock: Mutex::new(()),
        })
    }

    /// Upload a local file with sudo privileges (legacy compatibility API).
    #[allow(dead_code)]
    pub fn upload_sudo_file(&self, local_path: &str, remote_path: &str) -> Result<()> {
        let content = std::fs::read(local_path)
            .with_context(|| format!("Failed to read local file: {local_path}"))?;
        self.write_file_with_sudo_bytes(&content, remote_path, "", None, None, None)
    }

    // Forwarding methods retained for backward compatibility.
    pub fn execute_command(&self, command: &str) -> Result<(i32, String, String)> {
        SshConnection::execute_command(self, command)
    }

    pub fn execute_sudo_command(
        &self,
        command: &str,
        sudo_user: &str,
    ) -> Result<(i32, String, String)> {
        SshConnection::execute_sudo_command(self, command, sudo_user)
    }

    pub fn write_file_with_sudo(
        &self,
        content: &str,
        remote_path: &str,
        sudo_user: &str,
        mode: Option<&str>,
        owner: Option<&str>,
        group: Option<&str>,
    ) -> Result<()> {
        SshConnection::write_file_with_sudo(
            self,
            content,
            remote_path,
            sudo_user,
            mode.map(str::to_string),
            owner.map(str::to_string),
            group.map(str::to_string),
        )
    }

    pub fn write_file_content(&self, remote_path: &str, content: &str) -> Result<()> {
        SshConnection::write_file_content(self, remote_path, content)
    }

    pub fn upload_file(&self, local_path: &str, remote_path: &str) -> Result<()> {
        SshConnection::upload_file(self, local_path, remote_path)
    }
}

impl SshConnection for SshClient {
    fn execute_command(&self, command: &str) -> Result<(i32, String, String)> {
        self.execute_with_input(command, None)
    }

    fn execute_command_with_input(
        &self,
        command: &str,
        input: &[u8],
    ) -> Result<(i32, String, String)> {
        self.execute_with_input(command, Some(input))
    }

    fn execute_sudo_command(
        &self,
        command: &str,
        sudo_user: &str,
    ) -> Result<(i32, String, String)> {
        self.execute_sudo_command_with_input(command, sudo_user, &[])
    }

    fn execute_sudo_command_with_input(
        &self,
        command: &str,
        sudo_user: &str,
        input: &[u8],
    ) -> Result<(i32, String, String)> {
        if let Some(password_input) = sudo_password_input(&self.sudo_password)? {
            let validation = build_sudo_validation_command(sudo_user);
            let (code, _, stderr) = self.execute_with_input(&validation, Some(&password_input))?;
            if code != 0 {
                bail!("Remote sudo authentication failed: {}", stderr.trim());
            }
        }
        let sudo_command = build_noninteractive_sudo_command(command, sudo_user);
        debug!("Executing a sudo command on {}", self.host);
        self.execute_with_input(&sudo_command, Some(input))
    }

    fn write_file_with_sudo(
        &self,
        content: &str,
        remote_path: &str,
        sudo_user: &str,
        mode: Option<String>,
        owner: Option<String>,
        group: Option<String>,
    ) -> Result<()> {
        self.write_file_bytes_with_sudo(
            content.as_bytes(),
            remote_path,
            sudo_user,
            mode,
            owner,
            group,
        )
    }

    fn write_file_content(&self, remote_path: &str, content: &str) -> Result<()> {
        self.write_file_bytes(remote_path, content.as_bytes())
    }

    fn write_file_bytes(&self, remote_path: &str, content: &[u8]) -> Result<()> {
        debug!("Writing remote file content through SFTP");
        self.write_remote_bytes(remote_path, content, 0o644, false)
    }

    fn write_file_bytes_with_sudo(
        &self,
        content: &[u8],
        remote_path: &str,
        sudo_user: &str,
        mode: Option<String>,
        owner: Option<String>,
        group: Option<String>,
    ) -> Result<()> {
        self.write_file_with_sudo_bytes(content, remote_path, sudo_user, mode, owner, group)
    }

    fn read_file_bytes(&self, path: &str) -> Result<Option<Vec<u8>>> {
        let _operation = self
            .operation_lock
            .lock()
            .map_err(|_| anyhow!("SSH operation lock was poisoned"))?;
        let sftp = self
            .session
            .sftp()
            .context("Failed to initialize SFTP subsystem")?;
        let mut file = match sftp.open(Path::new(path)) {
            Ok(file) => file,
            Err(error) => {
                let io_error: io::Error = error.into();
                if io_error.kind() == io::ErrorKind::NotFound {
                    return Ok(None);
                }
                return Err(io_error).context("Failed to open remote file through SFTP");
            }
        };
        let mut content = Vec::new();
        let mut bounded = (&mut file).take((MAX_COMMAND_OUTPUT_BYTES + 1) as u64);
        bounded
            .read_to_end(&mut content)
            .context("Failed to read remote file through SFTP")?;
        if content.len() > MAX_COMMAND_OUTPUT_BYTES {
            bail!(
                "Remote file exceeded the {} byte safety limit",
                MAX_COMMAND_OUTPUT_BYTES
            );
        }
        Ok(Some(content))
    }

    fn read_file_bytes_with_sudo(&self, path: &str, sudo_user: &str) -> Result<Option<Vec<u8>>> {
        let inspect = test_path_exists_command(path);
        let (exists, _, stderr) = self.execute_sudo_command(&inspect, sudo_user)?;
        match exists {
            0 => {}
            1 => return Ok(None),
            code => bail!(
                "Failed to inspect privileged remote file (exit {}): {}",
                code,
                stderr.trim()
            ),
        }
        let command = format!("cat -- {}", shell_quote(path));
        if let Some(password_input) = sudo_password_input(&self.sudo_password)? {
            let validation = build_sudo_validation_command(sudo_user);
            let (code, _, stderr) = self.execute_with_input(&validation, Some(&password_input))?;
            if code != 0 {
                bail!("Remote sudo authentication failed: {}", stderr.trim());
            }
        }
        let sudo_command = build_noninteractive_sudo_command(&command, sudo_user);
        let (code, stdout, stderr) = self.execute_with_input_bytes(&sudo_command, None)?;
        if code != 0 {
            bail!(
                "Failed to read privileged remote file (exit {}): {}",
                code,
                String::from_utf8_lossy(&stderr).trim()
            );
        }
        Ok(Some(stdout))
    }

    fn upload_file(&self, local_path: &str, remote_path: &str) -> Result<()> {
        let mut local_file = File::open(local_path)
            .with_context(|| format!("Failed to open local file: {local_path}"))?;
        let _operation = self
            .operation_lock
            .lock()
            .map_err(|_| anyhow!("SSH operation lock was poisoned"))?;
        let sftp = self
            .session
            .sftp()
            .context("Failed to initialize SFTP subsystem")?;
        let mut remote_file = sftp
            .open_mode(
                Path::new(remote_path),
                OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE,
                0o644,
                OpenType::File,
            )
            .context("Failed to open remote upload path through SFTP")?;

        io::copy(&mut local_file, &mut remote_file)
            .context("Failed to transfer file contents through SFTP")?;
        remote_file
            .flush()
            .context("Failed to flush remote SFTP upload")?;
        remote_file
            .close()
            .context("Failed to close remote SFTP upload")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::create_test_host;

    #[test]
    fn shell_quote_preserves_literal_values() {
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("simple"), "'simple'");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), "'it'\"'\"'s'");
        assert_eq!(
            shell_quote("$(touch /tmp/pwn); *\nnext"),
            "'$(touch /tmp/pwn); *\nnext'"
        );
    }

    #[test]
    fn sudo_command_quotes_user_and_program_without_a_password() {
        let command = build_noninteractive_sudo_command("printf '%s' hello; id", "user; id");
        assert_eq!(
            command,
            "sudo -n -p '' -u 'user; id' -- sh -c 'printf '\"'\"'%s'\"'\"' hello; id'"
        );
        assert!(!command.contains("password"));

        assert_eq!(
            build_noninteractive_sudo_command("id", "root"),
            "sudo -n -p '' -- sh -c 'id'"
        );
    }

    #[test]
    fn sudo_password_is_a_separate_single_input_line() {
        assert_eq!(sudo_password_input("").unwrap(), None);
        assert_eq!(
            sudo_password_input("not in argv").unwrap(),
            Some(b"not in argv\n".to_vec())
        );
        assert!(sudo_password_input("two\nlines").is_err());
    }

    #[test]
    fn path_existence_probe_is_posix_and_shell_safe() {
        for path in ["/missing", "-leading", "with space", "it's; echo unsafe"] {
            let command = test_path_exists_command(path);
            assert!(!command.contains("test -e --"));
            let status = Command::new("sh").args(["-c", &command]).status().unwrap();
            assert_eq!(
                status.code(),
                Some(1),
                "probe had a syntax error: {command}"
            );
        }
        assert_eq!(test_path_exists_command("it's"), "test -e 'it'\"'\"'s'");
    }

    #[test]
    fn host_key_checking_is_strict_by_default_and_explicitly_configurable() {
        let mut host = Host::new("example.com");
        assert!(host_key_checking_enabled(&host).unwrap());

        host.set_variable("ansible_host_key_checking", "false");
        assert!(!host_key_checking_enabled(&host).unwrap());

        host.set_variable("ansible_host_key_checking", "maybe");
        assert!(host_key_checking_enabled(&host).is_err());
    }

    #[test]
    fn ssh_timeout_is_bounded_and_positive() {
        let mut host = Host::new("example.com");
        assert_eq!(
            ssh_timeout(&host).unwrap(),
            Duration::from_secs(DEFAULT_SSH_TIMEOUT_SECS)
        );

        host.set_variable("ansible_ssh_timeout", "3");
        assert_eq!(ssh_timeout(&host).unwrap(), Duration::from_secs(3));

        host.set_variable("ansible_ssh_timeout", "0");
        assert!(ssh_timeout(&host).is_err());
    }

    #[test]
    fn command_timeout_has_a_separate_safe_default() {
        let mut host = Host::new("example.com");
        assert_eq!(
            command_timeout(&host).unwrap(),
            Duration::from_secs(DEFAULT_COMMAND_TIMEOUT_SECS)
        );

        host.set_variable("ansible_command_timeout", "45");
        assert_eq!(command_timeout(&host).unwrap(), Duration::from_secs(45));
    }

    #[test]
    fn command_output_has_a_hard_memory_limit() {
        let stdout = vec![0_u8; MAX_COMMAND_OUTPUT_BYTES - 1];
        let mut stderr = Vec::new();
        let mut combined_size = stdout.len();
        append_bounded(&mut stderr, &[1], &mut combined_size).unwrap();
        assert!(append_bounded(&mut stderr, &[1], &mut combined_size).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn local_connection_drains_large_stdout_and_stderr_concurrently() {
        let host = Host::new("localhost");
        let connection = LocalConnection::new(&host).unwrap();
        let command = "i=0; while [ $i -lt 5000 ]; do printf '0123456789abcdef' >&1; printf 'fedcba9876543210' >&2; i=$((i+1)); done";
        let (code, stdout, stderr) = connection.execute_command(command).unwrap();
        assert_eq!(code, 0);
        assert_eq!(stdout.len(), 80_000);
        assert_eq!(stderr.len(), 80_000);
    }

    #[test]
    #[cfg(unix)]
    fn local_connection_passes_payload_via_stdin() {
        let host = Host::new("localhost");
        let connection = LocalConnection::new(&host).unwrap();
        let (code, stdout, stderr) = connection
            .execute_command_with_input("cat", b"payload; not shell syntax\n")
            .unwrap();
        assert_eq!(code, 0);
        assert_eq!(stdout, "payload; not shell syntax\n");
        assert!(stderr.is_empty());
    }

    #[test]
    #[cfg(unix)]
    fn local_connection_drains_output_while_writing_large_stdin() {
        const PAYLOAD_SIZE: usize = 2 * 1024 * 1024;

        let mut host = Host::new("localhost");
        host.set_variable("rustsible_command_timeout", "10");
        let connection = LocalConnection::new(&host).unwrap();
        let input = vec![b'x'; PAYLOAD_SIZE];
        // The child deliberately fills stdout beyond normal pipe capacity
        // before reading stdin. Sequential write-then-read implementations
        // deadlock here because both sides wait for the other pipe to drain.
        let command = format!(
            "head -c {PAYLOAD_SIZE} /dev/zero; head -c {PAYLOAD_SIZE} /dev/zero >&2; wc -c"
        );

        let (code, stdout, stderr) = connection
            .execute_command_with_input(&command, &input)
            .unwrap();

        assert_eq!(code, 0);
        assert!(stdout.as_bytes()[..PAYLOAD_SIZE]
            .iter()
            .all(|byte| *byte == 0));
        assert_eq!(&stdout[PAYLOAD_SIZE..], format!("{PAYLOAD_SIZE}\n"));
        assert_eq!(stderr.len(), PAYLOAD_SIZE);
        assert!(stderr.as_bytes().iter().all(|byte| *byte == 0));
    }

    #[test]
    #[cfg(unix)]
    fn local_connection_enforces_command_timeout() {
        let mut host = Host::new("localhost");
        host.set_variable("rustsible_command_timeout", "1");
        let connection = LocalConnection::new(&host).unwrap();
        let started = Instant::now();
        let result = connection.execute_command("sleep 5");
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(4));
    }

    #[test]
    #[cfg(unix)]
    fn local_connection_closes_pipes_inherited_by_background_descendants() {
        let mut host = Host::new("localhost");
        host.set_variable("rustsible_command_timeout", "1");
        let connection = LocalConnection::new(&host).unwrap();
        let started = Instant::now();

        let (code, _, _) = connection.execute_command("sleep 30 & exit 0").unwrap();

        assert_eq!(code, 0);
        assert!(started.elapsed() < Duration::from_secs(4));
    }

    #[test]
    #[cfg(unix)]
    fn local_connection_deadline_covers_detached_descendant_pipes() {
        let mut host = Host::new("localhost");
        host.set_variable("rustsible_command_timeout", "1");
        let connection = LocalConnection::new(&host).unwrap();
        let started = Instant::now();

        let result = connection.execute_command("setsid sleep 30 & exit 0");

        // Depending on the platform's `setsid`, the detached child either
        // closes inherited pipes promptly or keeps them open until our
        // deadline. Both are safe; the parent must never wait for `sleep 30`.
        if let Err(error) = result {
            assert!(error.to_string().contains("timed out"));
        }
        assert!(started.elapsed() < Duration::from_secs(4));
    }

    #[test]
    #[cfg(unix)]
    fn local_connection_terminates_an_unbounded_output_producer() {
        let host = Host::new("localhost");
        let connection = LocalConnection::new(&host).unwrap();
        let started = Instant::now();
        let result = connection.execute_command("while :; do printf '0123456789abcdef'; done");
        assert!(result.is_err());
        // Coverage instrumentation makes the tight output loop several times
        // slower. This bound still proves cleanup finishes promptly instead of
        // waiting for the normal command timeout.
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn explicit_ssh_localhost_is_not_misclassified_as_local() {
        let mut host = Host::new("localhost");
        assert_eq!(connection_kind(&host).unwrap(), ConnectionKind::Local);

        host.set_variable("ansible_connection", "ssh");
        assert_eq!(connection_kind(&host).unwrap(), ConnectionKind::Ssh);
    }

    #[test]
    fn local_connection_reads_and_writes_arbitrary_bytes() {
        let host = Host::new("localhost");
        let connection = LocalConnection::new(&host).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("binary content");
        let bytes = [0_u8, 0xff, b'\n', b'\'', 0x80];

        connection
            .write_file_bytes(path.to_str().unwrap(), &bytes)
            .unwrap();
        assert_eq!(
            connection.read_file_bytes(path.to_str().unwrap()).unwrap(),
            Some(bytes.to_vec())
        );
    }

    #[test]
    fn test_ssh_connection_trait_mock() {
        let mut mock = MockSshConnection::new();
        mock.expect_execute_command()
            .with(mockall::predicate::eq("echo hello"))
            .times(1)
            .returning(|_| Ok((0, "hello\n".to_string(), "".to_string())));

        let result = mock.execute_command("echo hello");
        assert!(result.is_ok());
        let (code, stdout, stderr) = result.unwrap();
        assert_eq!(code, 0);
        assert_eq!(stdout, "hello\n");
        assert_eq!(stderr, "");
    }

    #[test]
    fn test_ssh_client_struct() {
        fn assert_implements_ssh_connection<T: SshConnection>() {}
        assert_implements_ssh_connection::<SshClient>();
    }

    #[test]
    fn test_connect_invalid_host() {
        let host = create_test_host(
            "invalid",
            "invalid.example.com",
            22,
            Some("user"),
            Some("pass"),
        );
        let result = SshClient::connect(&host);
        assert!(result.is_err());
    }
}
