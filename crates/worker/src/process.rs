use anyhow::{bail, Context, Result};
use rustsible_protocol::ModuleResult;
use std::{
    fs::OpenOptions,
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{fs::OpenOptionsExt, process::CommandExt},
    },
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
const MAX_OUTPUT: usize = 16 * 1024 * 1024;

pub fn execute(program: &str, shell: bool, scratch: &Path) -> Result<ModuleResult> {
    let mut command = if shell {
        // The user's program is sensitive request data, not a worker argv
        // parameter. A private file also leaves stdin available for module IO.
        let path = scratch.join("program.sh");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(program.as_bytes())?;
        let mut command = Command::new("sh");
        command.arg(path);
        command
    } else {
        let argv = rustsible_protocol::tokenize_args(program)?;
        if argv.is_empty() {
            bail!("Command requires an executable");
        }
        let mut command = Command::new(&argv[0]);
        command.args(&argv[1..]);
        command
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn().context("Cannot start requested program")?;
    let mut out = child.stdout.take().context("Missing stdout pipe")?;
    let mut err = child.stderr.take().context("Missing stderr pipe")?;
    for descriptor in [out.as_raw_fd(), err.as_raw_fd()] {
        let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
        if flags < 0
            || unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
        {
            let error = io::Error::last_os_error();
            let _ = child.kill();
            let _ = child.wait();
            return Err(error).context("Cannot configure process pipes");
        }
    }
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut out_done = false;
    let mut err_done = false;
    let mut status = None;
    let mut finished_at = None;
    loop {
        for (reader, buffer, done) in [
            (&mut out as &mut dyn Read, &mut stdout, &mut out_done),
            (&mut err as &mut dyn Read, &mut stderr, &mut err_done),
        ] {
            if *done {
                continue;
            }
            let mut bytes = [0; 16 * 1024];
            match reader.read(&mut bytes) {
                Ok(0) => *done = true,
                Ok(length) => buffer.extend_from_slice(&bytes[..length]),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error).context("Cannot read program output");
                }
            }
        }
        if stdout.len() + stderr.len() > MAX_OUTPUT {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(ModuleResult { failed: true, uncertain: true,
                stdout: String::from_utf8_lossy(&stdout).into_owned(), stderr: String::from_utf8_lossy(&stderr).into_owned(),
                msg: "Program output exceeded the 16 MiB bound; modification outcome cannot be confirmed".into(), ..ModuleResult::default() });
        }
        if status.is_none() {
            status = child.try_wait()?;
            if status.is_some() {
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                finished_at = Some(Instant::now());
            }
        }
        if status.is_some()
            && ((out_done && err_done)
                || finished_at.is_some_and(|time| time.elapsed() > Duration::from_millis(100)))
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let status = status.context("Missing process status")?;
    let rc = status.code().unwrap_or_else(|| {
        use std::os::unix::process::ExitStatusExt;
        128 + status.signal().unwrap_or(0)
    });
    Ok(ModuleResult {
        changed: true,
        failed: rc != 0,
        rc: Some(rc),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        msg: format!("Program completed (exit {rc})"),
        ..ModuleResult::default()
    })
}
