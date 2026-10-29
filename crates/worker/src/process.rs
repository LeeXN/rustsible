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

pub fn execute(
    args: &rustsible_protocol::CommandArgs,
    shell: bool,
    check: bool,
    scratch: &Path,
) -> Result<ModuleResult> {
    let base = args
        .chdir
        .as_deref()
        .map(Path::new)
        .unwrap_or(Path::new("."));
    if !base.is_dir() {
        bail!("Command chdir is not an accessible directory");
    }
    let blocked = if let Some(path) = &args.creates {
        matches_path(&base.join(path))?
    } else {
        false
    };
    let blocked = blocked
        || if let Some(path) = &args.removes {
            !matches_path(&base.join(path))?
        } else {
            false
        };
    if blocked || check {
        return Ok(ModuleResult {
            changed: !blocked && args.has_prediction(),
            skipped: check && !args.has_prediction(),
            rc: Some(0),
            msg: if blocked {
                "Command guard prevented execution"
            } else {
                "Check mode: command was not executed"
            }
            .into(),
            ..ModuleResult::default()
        });
    }
    let mut command = if shell {
        // The user's program is sensitive request data, not a worker argv
        // parameter. A private file also leaves stdin available for module IO.
        let path = scratch.join("program.sh");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(
            args.cmd
                .as_deref()
                .context("Shell requires cmd")?
                .as_bytes(),
        )?;
        let mut command = Command::new("sh");
        command.arg(path);
        command
    } else {
        let mut argv = args.arguments()?;
        if args.expand_argument_vars {
            let variables = regex::Regex::new(r"\$(\w+|\{[^}]+\})")?;
            for argument in &mut argv {
                *argument = expand_argument(argument, &variables)?;
            }
        }
        if argv.is_empty() {
            bail!("Command requires an executable");
        }
        let mut command = Command::new(&argv[0]);
        command.args(&argv[1..]);
        command
    };
    command.current_dir(base);
    let input = args.stdin.as_ref().map(|input| {
        if args.stdin_add_newline {
            format!("{input}\n").into_bytes()
        } else {
            input.as_bytes().to_vec()
        }
    });
    let mut result = run(command, input.as_deref(), true, false)?;
    if args.strip_empty_ends {
        result.stdout = result.stdout.trim_end_matches(['\r', '\n']).to_owned();
        result.stderr = result.stderr.trim_end_matches(['\r', '\n']).to_owned();
    }
    Ok(result)
}

fn expand_argument(argument: &str, variables: &regex::Regex) -> Result<String> {
    let mut argument = argument.to_owned();
    if let Some(rest) = argument.strip_prefix('~') {
        let (user, tail) = rest
            .split_once('/')
            .map_or((rest, ""), |(user, _)| (user, &rest[user.len()..]));
        let home = if user.is_empty() {
            std::env::var("HOME").ok()
        } else {
            let name = std::ffi::CString::new(user)?;
            // Module execution is a single request process. Copy libc's NSS
            // record before another call can replace its static storage.
            let record = unsafe { libc::getpwnam(name.as_ptr()) };
            if record.is_null() {
                None
            } else {
                Some(
                    unsafe { std::ffi::CStr::from_ptr((*record).pw_dir) }
                        .to_string_lossy()
                        .into_owned(),
                )
            }
        };
        if let Some(home) = home {
            argument = format!("{home}{tail}");
        }
    }
    Ok(variables
        .replace_all(&argument, |captures: &regex::Captures<'_>| {
            let name = captures[1].trim_start_matches('{').trim_end_matches('}');
            std::env::var(name).unwrap_or_else(|_| captures[0].to_owned())
        })
        .into_owned())
}

fn matches_path(path: &Path) -> Result<bool> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let path = CString::new(path.as_os_str().as_bytes())?;
    // POSIX glob supplies the platform's path matching without a shell. The
    // zero-initialized result is released even after a partial allocation.
    let mut matches: libc::glob_t = unsafe { std::mem::zeroed() };
    extern "C" fn on_error(_: *const libc::c_char, error: libc::c_int) -> libc::c_int {
        // A missing parent is a non-match; permission and IO failures must not
        // be mistaken for permission to execute a modifying command.
        i32::from(!matches!(error, libc::ENOENT | libc::ENOTDIR))
    }
    let rc = unsafe {
        libc::glob(
            path.as_ptr(),
            libc::GLOB_NOSORT,
            Some(on_error),
            &mut matches,
        )
    };
    let found = matches.gl_pathc != 0;
    unsafe { libc::globfree(&mut matches) };
    match rc {
        0 | libc::GLOB_NOMATCH => Ok(found),
        _ => bail!("Cannot inspect command guard pattern"),
    }
}

/// All system tools and user commands share bounded duplex IO. A tool's input
/// stays on stdin; only the shell module chooses a shell interpreter.
pub fn run(
    mut command: Command,
    input: Option<&[u8]>,
    mutation: bool,
    system_tool: bool,
) -> Result<ModuleResult> {
    if mutation {
        rustsible_protocol::change::ensure_mutation_allowed()?;
    }
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn().context("Cannot start requested program")?;
    let mut writer = child.stdin.take();
    let mut position = 0;
    let mut out = child.stdout.take().context("Missing stdout pipe")?;
    let mut err = child.stderr.take().context("Missing stderr pipe")?;
    for descriptor in [
        Some(out.as_raw_fd()),
        Some(err.as_raw_fd()),
        writer.as_ref().map(AsRawFd::as_raw_fd),
    ]
    .into_iter()
    .flatten()
    {
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
        if let (Some(pipe), Some(bytes)) = (writer.as_mut(), input) {
            if position < bytes.len() {
                let end = (position + 65536).min(bytes.len());
                match pipe.write(&bytes[position..end]) {
                    Ok(0) => {
                        writer.take();
                    }
                    Ok(length) => position += length,
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) => {}
                    Err(error) if error.kind() == io::ErrorKind::BrokenPipe => {
                        writer.take();
                    }
                    Err(error) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(error).context("Cannot write program input");
                    }
                }
            }
            if position == bytes.len() {
                writer.take();
            }
        }
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
            return Ok(ModuleResult { failed: true, uncertain: mutation,
                stdout: String::from_utf8_lossy(&stdout).into_owned(), stderr: String::from_utf8_lossy(&stderr).into_owned(),
                msg: "Program output exceeded the 16 MiB bound; modification outcome cannot be confirmed".into(), ..ModuleResult::default() });
        }
        if status.is_none() {
            status = child.try_wait()?;
            if status.is_some() {
                if !system_tool {
                    unsafe {
                        libc::kill(-(child.id() as i32), libc::SIGKILL);
                    }
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
        changed: mutation && !system_tool,
        uncertain: mutation && rc == 0 && input.is_some_and(|bytes| position != bytes.len()),
        failed: rc != 0 || input.is_some_and(|bytes| position != bytes.len()),
        rc: Some(rc),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        msg: format!("Program completed (exit {rc})"),
        ..ModuleResult::default()
    })
}
