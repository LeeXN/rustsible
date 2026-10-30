use anyhow::{anyhow, bail, Context, Result};
use log::{debug, info, warn};
#[cfg(test)]
use mockall::automock;
use ssh2::{CheckResult, HashType, KnownHostFileKind, Session, Stream};
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::config::ConnectionConfig;
use crate::inventory::Host;

/// Preserve error classification through anyhow contexts without retaining
/// command text or stdin data. Once execution starts, a lost response cannot
/// safely be interpreted as proof that no state change occurred.
#[derive(Debug)]
pub(crate) struct TransportFailure {
    pub remote: bool,
    pub started: bool,
    message: String,
}
impl std::fmt::Display for TransportFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for TransportFailure {}
pub(crate) fn transport_error(error: anyhow::Error, remote: bool, started: bool) -> anyhow::Error {
    let message = error.to_string();
    error.context(TransportFailure {
        remote,
        started,
        message,
    })
}

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
    Compiled,
    Local,
    Ssh,
}

/// Resolve the transport without silently overriding an explicit inventory
/// choice. Bare localhost aliases remain convenient and use the local backend,
/// while `ansible_connection=ssh` always means SSH (including for localhost).
pub fn connection_kind(host: &Host) -> Result<ConnectionKind> {
    connection_kind_config(&ConnectionConfig::from_host(host)?)
}

fn connection_kind_config(host: &ConnectionConfig) -> Result<ConnectionKind> {
    if let Some(connection) = host
        .get_variable("ansible_connection")
        .or_else(|| host.get_variable("ansible_connection_type"))
    {
        return match connection.trim().to_ascii_lowercase().as_str() {
            "local" => Ok(ConnectionKind::Local),
            "ssh" | "smart" | "paramiko" => Ok(ConnectionKind::Ssh),
            other
                if crate::extensions::Registry::current()
                    .components()
                    .connections
                    .contains_key(other) =>
            {
                Ok(ConnectionKind::Compiled)
            }
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

fn host_key_checking_enabled(host: &ConnectionConfig) -> Result<bool> {
    for name in [
        "rustsible_host_key_checking",
        "ansible_host_key_checking",
        "ansible_ssh_host_key_checking",
    ] {
        if let Some(value) = host.get_variable(name) {
            return parse_inventory_bool(name, &value);
        }
    }

    Ok(true)
}

fn ssh_timeout(host: &ConnectionConfig) -> Result<Duration> {
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

pub(crate) fn command_timeout(host: &ConnectionConfig) -> Result<Duration> {
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

fn known_hosts_paths(host: &ConnectionConfig) -> Result<(Vec<PathBuf>, bool)> {
    if let Some(path) = host
        .get_variable("rustsible_known_hosts_file")
        .or_else(|| host.get_variable("ansible_ssh_known_hosts_file"))
    {
        if path.trim().is_empty() {
            bail!("The configured known_hosts file path cannot be empty");
        }
        return Ok((vec![PathBuf::from(path.as_ref())], true));
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

fn verify_host_key(session: &Session, host: &ConnectionConfig) -> Result<()> {
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
pub(crate) fn spawn_local_shell(command: &str) -> io::Result<Child> {
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
pub(crate) fn spawn_local_shell(command: &str) -> io::Result<Child> {
    Command::new("cmd")
        .args(["/C", command])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn spawn_local_shell(command: &str) -> io::Result<Child> {
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
}

/// A trusted transport keeps the same bounded framed worker protocol. Dropping
/// its session must cancel synchronous work and release the remote bootstrap.
pub trait WorkerConnection: SshConnection {
    fn start_worker(&self, command: &str) -> Result<crate::worker::transport::Session>;
}
/// A connection selected for one inventory host.
pub enum Connection {
    Local(LocalConnection),
    Ssh(SshClient),
    Compiled(Box<dyn WorkerConnection>),
}

impl Connection {
    pub fn connect(host: &Host) -> Result<Self> {
        Self::from_config(&ConnectionConfig::from_host(host)?)
    }

    pub fn from_config(config: &ConnectionConfig) -> Result<Self> {
        match connection_kind_config(config)? {
            ConnectionKind::Compiled => {
                let registry = crate::extensions::Registry::current();
                let name = config
                    .get_variable("ansible_connection")
                    .or_else(|| config.get_variable("ansible_connection_type"))
                    .context("Missing compiled connection name")?;
                let factory = registry
                    .components()
                    .connections
                    .get(name.as_ref())
                    .context("Missing compiled connection factory")?;
                factory(config)
            }
            ConnectionKind::Local => Ok(Self::Local(LocalConnection::from_config(config)?)),
            ConnectionKind::Ssh => Ok(Self::Ssh(SshClient::from_config(config)?)),
        }
    }

    pub(crate) fn start_worker(&self, command: &str) -> Result<crate::worker::transport::Session> {
        match self {
            Self::Compiled(connection) => connection.start_worker(command),
            Self::Local(_) => crate::worker::transport::Session::local(command),
            Self::Ssh(client) => {
                client.session.set_blocking(false);
                let deadline = Instant::now() + client.command_timeout;
                let mut channel = loop {
                    match client.session.channel_session() {
                        Ok(channel) => break channel,
                        Err(error) => {
                            let error: io::Error = error.into();
                            if error.kind() != io::ErrorKind::WouldBlock {
                                return Err(error).context("Cannot open worker SSH channel");
                            }
                        }
                    }
                    if Instant::now() >= deadline {
                        bail!("Opening worker SSH channel timed out");
                    }
                    std::thread::sleep(IO_POLL_INTERVAL);
                };
                loop {
                    match channel.exec(command) {
                        Ok(()) => break,
                        Err(error) => {
                            let error: io::Error = error.into();
                            if error.kind() != io::ErrorKind::WouldBlock {
                                return Err(error).context("Cannot start worker bootstrap");
                            }
                        }
                    }
                    if Instant::now() >= deadline {
                        bail!("Starting worker bootstrap timed out");
                    }
                    std::thread::sleep(IO_POLL_INTERVAL);
                }
                Ok(crate::worker::transport::Session::Remote {
                    channel,
                    _session: client.session.clone(),
                    pending: std::collections::VecDeque::new(),
                })
            }
        }
    }

    pub fn as_connection(&self) -> &dyn SshConnection {
        match self {
            Self::Local(connection) => connection,
            Self::Ssh(connection) => connection,
            Self::Compiled(connection) => connection.as_ref(),
        }
    }
}

pub struct LocalConnection {
    host: String,
    command_timeout: Duration,
}

impl LocalConnection {
    pub fn new(host: &Host) -> Result<Self> {
        Self::from_config(&ConnectionConfig::from_host(host)?)
    }

    fn from_config(host: &ConnectionConfig) -> Result<Self> {
        if connection_kind_config(host)? != ConnectionKind::Local {
            bail!("Host '{}' is not configured for local execution", host.name);
        }
        Ok(Self {
            host: host.name.clone(),
            command_timeout: command_timeout(host)?,
        })
    }

    fn execute_with_input_bytes(
        &self,
        command: &str,
        input: Option<&[u8]>,
    ) -> Result<(i32, Vec<u8>, Vec<u8>)> {
        let mut started = false;
        self.execute_bytes(command, input, &mut started)
            .map_err(|error| transport_error(error, false, started))
    }

    fn execute_bytes(
        &self,
        command: &str,
        input: Option<&[u8]>,
        started: &mut bool,
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
        *started = true;

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
}

pub struct SshClient {
    session: Session,
    host: String,
    command_timeout: Duration,
    operation_lock: Mutex<()>,
}

impl SshClient {
    fn execute_with_input_bytes(
        &self,
        command: &str,
        input: Option<&[u8]>,
    ) -> Result<(i32, Vec<u8>, Vec<u8>)> {
        let mut started = false;
        self.execute_bytes(command, input, &mut started)
            .map_err(|error| transport_error(error, true, started))
    }

    fn execute_bytes(
        &self,
        command: &str,
        input: Option<&[u8]>,
        started: &mut bool,
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

        *started = true;
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

    pub fn connect(host: &Host) -> Result<Self> {
        Self::from_config(&ConnectionConfig::from_host(host)?)
    }

    fn from_config(host: &ConnectionConfig) -> Result<Self> {
        info!(
            "Connecting to host: {} ({}:{})",
            host.name, host.hostname, host.port
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

        let username = host.get_ssh_user().unwrap_or("root");

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
            command_timeout,
            operation_lock: Mutex::new(()),
        })
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
    fn host_key_checking_is_strict_by_default_and_explicitly_configurable() {
        let mut host = Host::new("example.com");
        assert!(ConnectionConfig::from_host(&host)
            .and_then(|config| host_key_checking_enabled(&config))
            .unwrap());

        host.set_variable("ansible_host_key_checking", "false");
        assert!(!ConnectionConfig::from_host(&host)
            .and_then(|config| host_key_checking_enabled(&config))
            .unwrap());

        host.set_variable("ansible_host_key_checking", "maybe");
        assert!(ConnectionConfig::from_host(&host)
            .and_then(|config| host_key_checking_enabled(&config))
            .is_err());
    }

    #[test]
    fn ssh_timeout_is_bounded_and_positive() {
        let mut host = Host::new("example.com");
        assert_eq!(
            ConnectionConfig::from_host(&host)
                .and_then(|config| ssh_timeout(&config))
                .unwrap(),
            Duration::from_secs(DEFAULT_SSH_TIMEOUT_SECS)
        );

        host.set_variable("ansible_ssh_timeout", "3");
        assert_eq!(
            ConnectionConfig::from_host(&host)
                .and_then(|config| ssh_timeout(&config))
                .unwrap(),
            Duration::from_secs(3)
        );

        host.set_variable("ansible_ssh_timeout", "0");
        assert!(ConnectionConfig::from_host(&host)
            .and_then(|config| ssh_timeout(&config))
            .is_err());
    }

    #[test]
    fn command_timeout_has_a_separate_safe_default() {
        let mut host = Host::new("example.com");
        assert_eq!(
            ConnectionConfig::from_host(&host)
                .and_then(|config| command_timeout(&config))
                .unwrap(),
            Duration::from_secs(DEFAULT_COMMAND_TIMEOUT_SECS)
        );

        host.set_variable("ansible_command_timeout", "45");
        assert_eq!(
            ConnectionConfig::from_host(&host)
                .and_then(|config| command_timeout(&config))
                .unwrap(),
            Duration::from_secs(45)
        );
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
