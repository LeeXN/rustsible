use anyhow::{bail, Context, Result};
use rustsible_protocol::{
    registry::Builtin, AsyncSpec, ModuleResult, Request, Response, MAX_REQUEST_BYTES,
    MAX_RESPONSE_BYTES,
};
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use std::{
    collections::HashMap,
    ffi::{CStr, CString},
    fs::{self, File, OpenOptions},
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd, RawFd},
        unix::{
            ffi::OsStringExt,
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
            process::CommandExt,
        },
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

const FORMAT: u32 = 1;
const STATUS: &str = "status.json";
const PROGRAM: &str = "worker";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u32,
    jid: String,
    uid: u32,
    started: bool,
    finished: bool,
    pid: u32,
    start_ticks: Option<u64>,
    no_log: bool,
    result: Option<ModuleResult>,
}

struct Directory {
    file: File,
    base: Option<File>,
    path: PathBuf,
    jid: String,
}
fn safe(file: &File, directory: bool, mode: u32) -> Result<()> {
    let metadata = file.metadata()?;
    if metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o7777 != mode
        || metadata.is_dir() != directory
        || (!directory && (!metadata.is_file() || metadata.nlink() != 1))
    {
        bail!("Unsafe async state ownership, permissions or file type");
    }
    Ok(())
}
fn home() -> Result<PathBuf> {
    let mut buffer = vec![0; 16384];
    loop {
        let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
        let mut output = std::ptr::null_mut();
        let status = unsafe {
            libc::getpwuid_r(
                libc::geteuid(),
                &mut entry,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut output,
            )
        };
        if status == libc::ERANGE && buffer.len() < 1024 * 1024 {
            buffer.resize(buffer.len() * 2, 0);
            continue;
        }
        if status != 0 || output.is_null() || entry.pw_dir.is_null() {
            bail!("Cannot determine execution identity home; configure ansible_async_dir");
        }
        return Ok(PathBuf::from(std::ffi::OsString::from_vec(
            unsafe { CStr::from_ptr(entry.pw_dir) }.to_bytes().to_vec(),
        )));
    }
}
fn base_path(directory: Option<&str>) -> Result<PathBuf> {
    let path = match directory {
        None => home()?.join(".rustsible_async"),
        Some(path) if path.starts_with("~/") => home()?.join(&path[2..]),
        Some(path) if Path::new(path).is_absolute() => PathBuf::from(path),
        _ => bail!("Async directory must be absolute or begin with ~/"),
    };
    Ok(path)
}
impl Directory {
    fn open(directory: Option<&str>, jid: &str, create: bool) -> Result<Self> {
        rustsible_protocol::validate_job_id(jid)?;
        let path = base_path(directory)?;
        if create {
            rustsible_protocol::change::ensure_mutation_allowed()?;
            match fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&path)
            {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error).context("Cannot create async state base"),
            }
        }
        let base = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(&path)?;
        safe(&base, true, 0o700)?;
        let name = CString::new(jid)?;
        if create && unsafe { libc::mkdirat(base.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
            return Err(std::io::Error::last_os_error())
                .context("Cannot create exclusive async job directory");
        }
        let descriptor = unsafe {
            libc::openat(
                base.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if descriptor < 0 {
            return Err(std::io::Error::last_os_error()).context("Cannot open async job directory");
        }
        let file = unsafe { File::from_raw_fd(descriptor) };
        safe(&file, true, 0o700)?;
        Ok(Self {
            file,
            base: Some(base),
            path: path.join(jid),
            jid: jid.to_owned(),
        })
    }
    fn inherited(descriptor: RawFd, jid: &str) -> Result<Self> {
        rustsible_protocol::validate_job_id(jid)?;
        if descriptor < 3 {
            bail!("Invalid async directory descriptor");
        }
        let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFD) };
        if flags < 0 {
            return Err(std::io::Error::last_os_error())
                .context("Invalid async directory descriptor");
        }
        let file = unsafe { File::from_raw_fd(descriptor) };
        safe(&file, true, 0o700)?;
        if flags < 0
            || unsafe { libc::fcntl(descriptor, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0
        {
            return Err(std::io::Error::last_os_error())
                .context("Cannot protect async directory descriptor");
        }
        let path = fs::read_link(format!("/proc/self/fd/{descriptor}"))?;
        Ok(Self {
            file,
            base: None,
            path,
            jid: jid.to_owned(),
        })
    }
    fn location(&self, name: &str) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}", self.file.as_raw_fd())).join(name)
    }
    fn load(&self) -> Result<State> {
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(self.location(STATUS))?;
        safe(&file, false, 0o600)?;
        if file.metadata()?.len() > MAX_RESPONSE_BYTES as u64 {
            bail!("Async result exceeds its size bound");
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(MAX_RESPONSE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            bail!("Async result exceeds its size bound");
        }
        let state: State = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow::anyhow!("Invalid async state JSON"))?;
        if state.version != FORMAT
            || state.jid != self.jid
            || state.uid != unsafe { libc::geteuid() }
            || (state.started && (state.pid == 0 || state.start_ticks.is_none()))
            || (state.finished && (!state.started || state.result.is_none()))
        {
            bail!("Invalid async state version, identity or result");
        }
        Ok(state)
    }
    fn save(&self, state: &State) -> Result<()> {
        let mut temporary = tempfile::Builder::new()
            .prefix(".status-")
            .tempfile_in(self.location(""))?;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
        serde_json::to_writer(temporary.as_file_mut(), state)?;
        if temporary.as_file().metadata()?.len() > MAX_RESPONSE_BYTES as u64 {
            bail!("Async result exceeds its size bound");
        }
        temporary.as_file().sync_all()?;
        temporary
            .persist(self.location(STATUS))
            .map_err(|error| error.error)?;
        self.file.sync_all()?;
        Ok(())
    }
    fn unlink(&self, name: &str) -> Result<()> {
        let name = CString::new(name)?;
        if unsafe { libc::unlinkat(self.file.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::NotFound {
                return Err(error.into());
            }
        }
        Ok(())
    }
    fn remove(&self) -> Result<()> {
        self.unlink(STATUS)?;
        self.unlink(PROGRAM)?;
        let base = self
            .base
            .as_ref()
            .context("Cleanup requires the job base")?;
        let name = CString::new(self.jid.as_str())?;
        if unsafe { libc::unlinkat(base.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) } != 0 {
            return Err(std::io::Error::last_os_error())
                .context("Cannot remove async job directory");
        }
        Ok(())
    }
    fn metadata(&self, started: bool, finished: bool) -> HashMap<String, Value> {
        HashMap::from([
            ("ansible_job_id".into(), Value::String(self.jid.clone())),
            ("started".into(), Value::Bool(started)),
            ("finished".into(), Value::Bool(finished)),
            (
                "results_file".into(),
                Value::String(self.path.join(STATUS).to_string_lossy().into_owned()),
            ),
        ])
    }
}

struct Startup<'a> {
    job: &'a Directory,
    retained: bool,
}
impl Drop for Startup<'_> {
    fn drop(&mut self) {
        if !self.retained {
            let _ = self.job.remove();
        }
    }
}

pub fn start(request: &Request) -> ModuleResult {
    match start_job(request) {
        Ok(result) => result,
        Err(error) => ModuleResult {
            failed: true,
            no_log: request
                .asynchronous
                .as_ref()
                .is_some_and(|specification| specification.no_log),
            msg: format!("Cannot start native async task: {error:#}"),
            ..Default::default()
        },
    }
}
fn start_job(request: &Request) -> Result<ModuleResult> {
    let specification = request
        .asynchronous
        .as_ref()
        .context("Missing async specification")?;
    request.validate()?;
    Builtin::lookup(&request.module)?.validate_parameters(&request.args)?;
    let job = Directory::open(specification.directory.as_deref(), &specification.jid, true)?;
    let mut startup = Startup {
        job: &job,
        retained: false,
    };
    let mut program = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o700)
        .custom_flags(libc::O_NOFOLLOW)
        .open(job.location(PROGRAM))?;
    std::io::copy(&mut File::open("/proc/self/exe")?, &mut program)?;
    program.sync_all()?;
    drop(program);
    job.save(&State {
        version: FORMAT,
        jid: job.jid.clone(),
        uid: unsafe { libc::geteuid() },
        started: false,
        finished: false,
        pid: 0,
        start_ticks: None,
        no_log: specification.no_log,
        result: None,
    })?;
    let descriptor = job.file.as_raw_fd();
    let mut command = Command::new(job.location(PROGRAM));
    command
        .args(["--async-supervisor", &descriptor.to_string(), &job.jid])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // The launcher is reaped by its caller. The supervisor adopts the pinned
    // directory descriptor and outlives the originating SSH/worker session.
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let child = libc::fork();
            if child < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if child > 0 {
                libc::_exit(0);
            }
            libc::prctl(libc::PR_SET_PDEATHSIG, 0, 0, 0, 0);
            let flags = libc::fcntl(descriptor, libc::F_GETFD);
            if flags < 0 || libc::fcntl(descriptor, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .context("Cannot execute retained async worker; check async directory noexec policy")?;
    let mut input = child.stdin.take().context("Missing async input")?;
    rustsible_protocol::write_frame(&mut input, request, MAX_REQUEST_BYTES)?;
    drop(input);
    startup.retained = true;
    let ready = rustsible_protocol::read_frame::<Response>(
        &mut child
            .stdout
            .take()
            .context("Missing async readiness pipe")?,
        MAX_RESPONSE_BYTES,
    );
    let _ = child.wait();
    match ready.and_then(Response::validate) {
        Ok(mut result) => {
            result.no_log |= specification.no_log;
            Ok(result)
        }
        Err(error) => {
            let mut values = job.metadata(false, false);
            values.remove("started");
            values.remove("finished");
            Ok(ModuleResult {
                failed: true,
                uncertain: true,
                no_log: specification.no_log,
                values,
                msg: format!("Async startup result cannot be confirmed: {error:#}"),
                ..Default::default()
            })
        }
    }
}

fn start_ticks(pid: u32) -> Result<Option<u64>> {
    let text = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let fields = text.rsplit_once(')').context("Invalid process stat")?.1;
    if fields.split_whitespace().next() == Some("Z") {
        return Ok(None);
    }
    Ok(Some(
        fields
            .split_whitespace()
            .nth(19)
            .context("Missing process start time")?
            .parse()?,
    ))
}

pub fn supervise(descriptor: RawFd, jid: &str) -> Result<()> {
    let job = Directory::inherited(descriptor, jid)?;
    let mut request: Request =
        rustsible_protocol::read_frame(&mut std::io::stdin().lock(), MAX_REQUEST_BYTES)?;
    request.validate()?;
    let specification: AsyncSpec = request
        .asynchronous
        .take()
        .context("Missing async supervision specification")?;
    if specification.jid != jid {
        bail!("Async job identity mismatch");
    }
    Builtin::lookup(&request.module)?.validate_parameters(&request.args)?;
    crate::supervisor::prepare_detached()?;
    let mut state = job.load()?;
    state.started = true;
    state.pid = std::process::id();
    state.start_ticks = start_ticks(state.pid)?;
    job.save(&state)?;
    let launched = ModuleResult {
        changed: true,
        no_log: state.no_log,
        values: job.metadata(true, false),
        msg: "Native async task started".into(),
        ..Default::default()
    };
    // Once durable state exists, disconnecting the starter must not cancel a
    // detached task. A lost acknowledgement is resolved through async_status.
    let _ = rustsible_protocol::write_frame(
        &mut std::io::stdout().lock(),
        &Response::new(launched),
        MAX_RESPONSE_BYTES,
    );
    unsafe {
        let null = File::options().read(true).write(true).open("/dev/null")?;
        for fd in [0, 1, 2] {
            if libc::dup2(null.as_raw_fd(), fd) < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
    }
    request.timeout_ms = specification.timeout_ms;
    request.managed = false;
    let execution = crate::supervisor::run_detached(
        Duration::from_millis(specification.timeout_ms),
        true,
        matches!(request.module.as_str(), "service" | "systemd" | "package"),
        |scratch| crate::modules::execute(&request, scratch, None),
    )?;
    let mut result = execution.result;
    if execution.timed_out {
        // Unlike a lost controller response, the job supervisor has confirmed
        // termination and reaping of its entire owned tree before recording this.
        result.uncertain = false;
        result.failed = true;
        result.changed = true;
        result.values.insert("timed_out".into(), Value::Bool(true));
        result.msg =
            "Async task exceeded its runtime limit; owned descendants were terminated".into();
    }
    result.no_log |= state.no_log;
    state.result = Some(result);
    state.finished = true;
    job.save(&state)?;
    job.unlink(PROGRAM)?;
    Ok(())
}

pub fn status(args: &Value, check: bool) -> Result<ModuleResult> {
    Builtin::AsyncStatus.validate_parameters(args)?;
    let jid = args
        .get("jid")
        .and_then(Value::as_str)
        .context("Missing async job ID")?;
    let cleanup = args.get("mode").and_then(Value::as_str) == Some("cleanup");
    let job = match Directory::open(args.get("_async_dir").and_then(Value::as_str), jid, false) {
        Ok(job) => job,
        Err(error)
            if cleanup
                && error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(ModuleResult {
                msg: "Async state already absent".into(),
                ..Default::default()
            })
        }
        Err(error) => {
            return Err(error).context("Cannot open async job state for this execution identity")
        }
    };
    let state = job.load()?;
    if cleanup {
        if !state.finished {
            bail!("Cannot clean a running or unconfirmed async job");
        }
        if !check {
            rustsible_protocol::change::ensure_mutation_allowed()?;
            job.remove()?;
        }
        return Ok(ModuleResult {
            changed: true,
            no_log: state.no_log,
            msg: if check {
                "Would remove completed async state"
            } else {
                "Completed async state removed"
            }
            .into(),
            values: HashMap::from([("erased".into(), Value::String(jid.into()))]),
            ..Default::default()
        });
    }
    let mut result = if state.finished {
        state.result.context("Missing completed async result")?
    } else {
        let lost = state.started && start_ticks(state.pid)? != state.start_ticks;
        ModuleResult {
            failed: lost,
            uncertain: lost,
            msg: if lost {
                "Async supervisor exited without a confirmed result"
            } else {
                "Async task is running"
            }
            .into(),
            ..Default::default()
        }
    };
    result.no_log |= state.no_log;
    result
        .values
        .extend(job.metadata(state.started, state.finished));
    Ok(result)
}
