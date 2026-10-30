//! One native executable invocation, always inside a request supervisor. The
//! controller execs its own hidden runner first so a multithreaded controller
//! never forks and then allocates or locks in the child.
use anyhow::{bail, Context, Result};
use rustsible_protocol::{
    extension::{Call, Definition, Hello, Probe, Reply, API_VERSION, MAX_RESULT_BYTES},
    ModuleResult,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::OpenOptions,
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{MetadataExt, OpenOptionsExt},
            process::CommandExt,
        },
    },
    path::PathBuf,
    process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio},
    time::{Duration, Instant},
};
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invocation {
    pub program: PathBuf,
    pub sha256: String,
    pub definition: Definition,
    pub call: Call,
}
struct Pipes {
    child: Child,
    input: Option<ChildStdin>,
    output: ChildStdout,
    diagnostic: ChildStderr,
    diagnostic_bytes: usize,
    deadline: Instant,
}
impl Pipes {
    fn spawn(mut command: Command, timeout: Duration) -> Result<Self> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .context("Cannot start native extension process")?;
        let input = child.stdin.take().context("Missing extension input")?;
        let output = child.stdout.take().context("Missing extension output")?;
        let diagnostic = child
            .stderr
            .take()
            .context("Missing extension diagnostics")?;
        for fd in [
            input.as_raw_fd(),
            output.as_raw_fd(),
            diagnostic.as_raw_fd(),
        ] {
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
            {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::last_os_error().into());
            }
        }
        Ok(Self {
            child,
            input: Some(input),
            output,
            diagnostic,
            diagnostic_bytes: 0,
            deadline: Instant::now() + timeout,
        })
    }
    fn pump(&mut self) -> Result<()> {
        let mut buffer = [0u8; 4096];
        loop {
            match self.diagnostic.read(&mut buffer) {
                Ok(0) => break,
                Ok(length) => {
                    self.diagnostic_bytes += length;
                    if self.diagnostic_bytes > 65536 {
                        bail!("Native extension diagnostics exceeded 64 KiB");
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error.into()),
            }
        }
        if Instant::now() >= self.deadline {
            bail!("Native extension timed out");
        }
        Ok(())
    }
    fn send<T: Serialize>(&mut self, value: &T) -> Result<()> {
        let bytes = rustsible_protocol::encode(value, rustsible_protocol::MAX_REQUEST_BYTES)?;
        let mut remaining = bytes.as_slice();
        while !remaining.is_empty() {
            self.pump()?;
            match self
                .input
                .as_mut()
                .context("Extension input is closed")?
                .write(remaining)
            {
                Ok(0) => bail!("Native extension closed input"),
                Ok(length) => remaining = &remaining[length..],
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error.into()),
            }
            if !remaining.is_empty() {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        Ok(())
    }
    fn read_exact(&mut self, mut output: &mut [u8]) -> Result<()> {
        while !output.is_empty() {
            self.pump()?;
            match self.output.read(output) {
                Ok(0) => bail!("Native extension closed its result stream"),
                Ok(length) => output = &mut output[length..],
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error.into()),
            }
            if !output.is_empty() {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        Ok(())
    }
    fn receive<T: serde::de::DeserializeOwned>(&mut self) -> Result<T> {
        self.receive_limit(MAX_RESULT_BYTES)
    }
    fn receive_limit<T: serde::de::DeserializeOwned>(&mut self, limit: usize) -> Result<T> {
        let mut header = [0u8; 4];
        self.read_exact(&mut header)?;
        let length = u32::from_be_bytes(header) as usize;
        if length == 0 || length > limit {
            bail!("Native extension result exceeded its frame bound");
        }
        let mut frame = vec![0u8; length + 4];
        frame[..4].copy_from_slice(&header);
        self.read_exact(&mut frame[4..])?;
        rustsible_protocol::decode(&frame, limit)
            .map_err(|_| anyhow::anyhow!("Malformed native extension result"))
    }
    fn finish(&mut self) -> Result<()> {
        self.input.take();
        loop {
            self.pump()?;
            if let Some(status) = self.child.try_wait()? {
                if !status.success() {
                    bail!("Native extension process failed");
                }
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
impl Drop for Pipes {
    fn drop(&mut self) {
        self.input.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The caller's outer supervisor owns every descendant, including setsid and
/// double forks. A missing result after parameters were sent is uncertain for
/// a mutating module and is never replayed automatically.
pub fn execute(invocation: &Invocation) -> ModuleResult {
    let mut started = false;
    let result = (|| -> Result<ModuleResult> {
        invocation.definition.validate_call(&invocation.call)?;
        let mut program = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&invocation.program)
            .context("Cannot open native extension artifact")?;
        let metadata = program.metadata()?;
        if (metadata.uid() != 0 && metadata.uid() != unsafe { libc::geteuid() })
            || metadata.mode() & 0o022 != 0
        {
            bail!("Native extension artifact has unsafe ownership/write permissions");
        }
        rustsible_protocol::extension::verify_binary(
            &mut program,
            &invocation.sha256,
            std::env::consts::ARCH,
            "Native extension",
        )?;
        // CLOEXEC protects the descriptor. The child addresses the live parent
        // fd, as target Git and worker bootstrap already do for pinned paths.
        let path = format!("/proc/{}/fd/{}", std::process::id(), program.as_raw_fd());
        let mut command = Command::new(path);
        command.arg("--rustsible-extension");
        let mut pipes = Pipes::spawn(command, Duration::from_millis(invocation.call.timeout_ms))?;
        pipes.send(&Probe {
            api_version: API_VERSION,
            name: invocation.definition.name.clone(),
            kind: invocation.definition.kind,
        })?;
        let hello: Hello = pipes.receive()?;
        hello.validate(&invocation.definition)?;
        started = true;
        pipes.send(&invocation.call)?;
        let reply: Reply = pipes.receive()?;
        let result = reply.validate()?;
        pipes.finish()?;
        Ok(result)
    })();
    match result {
        Ok(mut result) => {
            result.no_log |= invocation.call.no_log;
            result
        }
        Err(error) => ModuleResult {
            failed: true,
            uncertain: started
                && invocation.definition.capabilities.mutates
                && !invocation.call.check,
            no_log: invocation.call.no_log,
            msg: format!(
                "Native extension {} failed: {error:#}",
                invocation.definition.name
            ),
            ..Default::default()
        },
    }
}

pub fn runner_main() -> Result<()> {
    let invocation: Invocation = rustsible_protocol::read_frame(
        &mut io::stdin().lock(),
        rustsible_protocol::MAX_REQUEST_BYTES,
    )?;
    invocation.definition.validate_call(&invocation.call)?;
    crate::supervisor::prepare()?;
    let result = crate::supervisor::run(
        Duration::from_millis(invocation.call.timeout_ms),
        false,
        invocation.definition.capabilities.mutates && !invocation.call.check,
        false,
        |_| execute(&invocation),
    )?;
    rustsible_protocol::write_frame(
        &mut io::stdout().lock(),
        &Reply {
            api_version: API_VERSION,
            result: result.result,
        },
        rustsible_protocol::MAX_RESPONSE_BYTES,
    )
}

/// Controller-side providers use the same process-tree supervision as target
/// modules without requiring a worker artifact or opening a target connection.
pub fn invoke_controller(invocation: &Invocation) -> Result<ModuleResult> {
    let mut command = Command::new(format!("/proc/{}/exe", std::process::id()));
    command.arg("--native-extension-runner");
    let mut pipes = Pipes::spawn(
        command,
        Duration::from_millis(invocation.call.timeout_ms) + Duration::from_secs(5),
    )?;
    pipes.send(invocation)?;
    let response: Reply = pipes.receive_limit(rustsible_protocol::MAX_RESPONSE_BYTES)?;
    let result = response.validate()?;
    pipes.finish()?;
    Ok(result)
}

/// Embedding controllers dispatch this before application argument parsing.
pub fn handle_runner_mode() -> Option<Result<()>> {
    (std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new("--native-extension-runner")))
    .then(runner_main)
}
