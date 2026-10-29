mod accounts;
mod facts;
mod files;
mod filesystem;
mod http;
mod jobs;
mod lines;
mod modules;
mod packages;
mod process;
mod services;
mod staging;
mod supervisor;
mod tools;
use anyhow::{bail, Result};
use rustsible_protocol::{
    Hello, ModuleResult, Request, Response, BUILD_ID, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES,
    NATIVE_MODULES, VERSION,
};
use std::{
    fs,
    io::{self, Read, Write},
    os::{
        fd::FromRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    time::Duration,
};

fn main() {
    // Neither panic diagnostics nor core files may expose a request's secrets.
    std::panic::set_hook(Box::new(|_| eprintln!("Native worker internal failure")));
    unsafe {
        let limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        libc::setrlimit(libc::RLIMIT_CORE, &limit);
    }
    if run().is_err() {
        eprintln!("Native worker protocol/execution failure");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    if std::env::consts::OS != "linux" {
        bail!("Native workers require Linux");
    }
    let mut args = std::env::args().skip(1);
    let action = args.next();
    if action.as_deref() == Some("--async-supervisor") {
        let descriptor = args
            .next()
            .ok_or_else(|| anyhow::anyhow!("Missing async descriptor"))?
            .parse()?;
        let jid = args
            .next()
            .ok_or_else(|| anyhow::anyhow!("Missing async job ID"))?;
        if args.next().is_some() {
            bail!("Unexpected async supervisor arguments");
        }
        return jobs::supervise(descriptor, &jid);
    }
    if args.next().is_some() || !matches!(action.as_deref(), Some("--handshake" | "--serve")) {
        bail!("Expected worker --handshake or --serve");
    }
    if std::env::var("RUSTSIBLE_BOOTSTRAP_DIR_FD").as_deref() == Ok("3") {
        // The bootstrap's directory handle must not leak into business tools.
        unsafe {
            libc::close(3);
        }
        std::env::remove_var("RUSTSIBLE_BOOTSTRAP_DIR_FD");
    }
    let _managed_directory = ManagedDirectory::detect()?;
    supervisor::prepare()?;
    let hello = Hello {
        version: VERSION,
        build_id: BUILD_ID.into(),
        os: "linux".into(),
        arch: std::env::consts::ARCH.into(),
        uid: unsafe { libc::geteuid() },
        modules: NATIVE_MODULES.iter().map(|name| (*name).into()).collect(),
    };
    let mut output = io::stdout().lock();
    rustsible_protocol::write_frame(&mut output, &hello, MAX_RESPONSE_BYTES)?;
    if action.as_deref() == Some("--handshake") {
        return Ok(());
    }
    // No buffering: the supervisor must see EOF as soon as the controller dies.
    let descriptor = unsafe { libc::fcntl(0, libc::F_DUPFD_CLOEXEC, 3) };
    if descriptor < 0 {
        return Err(io::Error::last_os_error().into());
    }
    let mut input = Input(unsafe { fs::File::from_raw_fd(descriptor) });
    loop {
        let mut first = [0];
        match input.read(&mut first) {
            Ok(0) => return Ok(()),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Ok(()),
            Err(error) => return Err(error.into()),
        }
        let request: Request = match rustsible_protocol::read_frame(
            &mut first.as_slice().chain(&mut input),
            MAX_REQUEST_BYTES,
        ) {
            Ok(request) => request,
            Err(_) => {
                rustsible_protocol::write_frame(
                    &mut output,
                    &Response::new(ModuleResult {
                        failed: true,
                        msg: "Invalid worker request frame".into(),
                        ..ModuleResult::default()
                    }),
                    MAX_RESPONSE_BYTES,
                )?;
                return Ok(());
            }
        };
        if let Err(error) = request.validate() {
            rustsible_protocol::write_frame(
                &mut output,
                &Response::new(ModuleResult {
                    failed: true,
                    msg: error.to_string(),
                    ..ModuleResult::default()
                }),
                MAX_RESPONSE_BYTES,
            )?;
            return Ok(());
        }
        let payload = if let Some(payload) = &request.payload {
            let executable = std::env::current_exe()?;
            let mut builder = tempfile::Builder::new();
            builder.prefix(".rustsible-payload-");
            let prepared = if request.managed {
                builder.tempfile_in(
                    executable
                        .parent()
                        .ok_or_else(|| anyhow::anyhow!("Worker has no parent"))?,
                )
            } else {
                builder.tempfile()
            };
            let mut file = match prepared {
                Ok(file) => file,
                Err(error) => {
                    rustsible_protocol::write_frame(
                        &mut output,
                        &Response::new(ModuleResult {
                            failed: true,
                            msg: format!("Cannot prepare private input staging: {error}"),
                            ..ModuleResult::default()
                        }),
                        MAX_RESPONSE_BYTES,
                    )?;
                    return Ok(());
                }
            };
            rustsible_protocol::write_frame(
                &mut output,
                &rustsible_protocol::PayloadReady::new(),
                MAX_RESPONSE_BYTES,
            )?;
            let received = receive_payload(&mut input, file.as_file_mut(), payload);
            if let Err(error) = received {
                rustsible_protocol::write_frame(&mut output, &Response::new(ModuleResult { failed: true, msg: format!("Cannot stage complete file input; business files were not modified: {error}"), ..ModuleResult::default() }), MAX_RESPONSE_BYTES)?;
                return Ok(());
            }
            Some(file)
        } else {
            None
        };
        let timeout = if request.module == "setup" {
            request.timeout_ms.min(
                request
                    .args
                    .get("gather_timeout")
                    .and_then(serde_yaml::Value::as_u64)
                    .unwrap_or(10)
                    .saturating_mul(1000),
            )
        } else {
            request.timeout_ms
        };
        let execution = supervisor::run(
            Duration::from_millis(timeout),
            request.managed,
            request.asynchronous.is_some()
                || rustsible_protocol::registry::Builtin::lookup(&request.module)?
                    .may_modify(&request.args, request.check),
            !request.check
                && (request.asynchronous.is_some()
                    || matches!(request.module.as_str(), "service" | "systemd" | "package")),
            |scratch| {
                if request.asynchronous.is_some() {
                    jobs::start(&request)
                } else {
                    modules::execute(&request, scratch, payload.as_ref().map(|file| file.path()))
                }
            },
        )?;
        rustsible_protocol::write_frame(
            &mut output,
            &Response::new(execution.result),
            MAX_RESPONSE_BYTES,
        )?;
        if execution.cancelled {
            return Ok(());
        }
    }
}

/// The bootstrap normally removes its directory. On abrupt SSH disconnect its
/// shell may die before EXIT runs, so the worker also owns a bounded cleanup.
struct ManagedDirectory(Option<fs::File>);
impl ManagedDirectory {
    fn detect() -> Result<Self> {
        let executable = std::env::current_exe()?;
        let directory = executable.parent().filter(|directory| {
            directory
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_prefix(".rustsible-worker-"))
                .is_some_and(|suffix| {
                    suffix.len() >= 10 && suffix.bytes().all(|byte| byte.is_ascii_alphanumeric())
                })
        });
        if let Some(directory) = directory {
            let file = fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
                .open(directory)?;
            let metadata = file.metadata()?;
            if metadata.is_dir()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.permissions().mode() & 0o777 == 0o700
            {
                return Ok(Self(Some(file)));
            }
        }
        Ok(Self(None))
    }
}
impl Drop for ManagedDirectory {
    fn drop(&mut self) {
        if let Some(file) = &self.0 {
            // Holding the directory open prevents inode reuse; current_exe
            // follows our executable's actual location after ancestor renames.
            // Never remove a replacement at a formerly valid pathname.
            let cleanup = (|| -> Result<()> {
                let executable = std::env::current_exe()?;
                let directory = executable
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("Worker executable has no parent"))?;
                let expected = file.metadata()?;
                let current = match fs::symlink_metadata(directory) {
                    Ok(current) => current,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                    Err(error) => return Err(error.into()),
                };
                if !current.is_dir()
                    || (current.dev(), current.ino()) != (expected.dev(), expected.ino())
                {
                    bail!("Worker directory identity changed");
                }
                fs::remove_dir_all(directory)?;
                Ok(())
            })();
            if cleanup.is_err() {
                eprintln!("Native worker private-directory cleanup failed");
            }
        }
    }
}
struct Input(fs::File);
impl Read for Input {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            if supervisor::interrupted() {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "Worker cancelled",
                ));
            }
            match self.0.read(buffer) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => return result,
            }
        }
    }
}

fn receive_payload(
    input: &mut impl Read,
    file: &mut fs::File,
    payload: &rustsible_protocol::Payload,
) -> Result<()> {
    use sha2::Digest;
    let mut remaining = payload.size;
    let mut digest = sha2::Sha256::new();
    let mut buffer = [0; 65536];
    while remaining != 0 {
        let length = remaining.min(buffer.len() as u64) as usize;
        let read = input.read(&mut buffer[..length])?;
        if read == 0 {
            bail!("Incomplete file payload");
        }
        file.write_all(&buffer[..read])?;
        digest.update(&buffer[..read]);
        remaining -= read as u64;
    }
    if format!("{:x}", digest.finalize()) != payload.sha256.to_ascii_lowercase() {
        bail!("Payload SHA-256 mismatch");
    }
    Ok(())
}
