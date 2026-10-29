//! A worker owns exactly one execution tree. Keeping this in a separate process
//! avoids changing the controller's signal handlers or adopting another host's
//! children. Even double-forked/setsid descendants are adopted and reaped here.
use anyhow::{bail, Context, Result};
use rustsible_protocol::{decode, write_frame, ModuleResult, Response, MAX_RESPONSE_BYTES};
use std::{
    fs::{self, File},
    io::{self, Read},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::net::UnixDatagram,
    },
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);
extern "C" fn interrupt(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::Relaxed);
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Supervised {
    pub result: ModuleResult,
    pub cancelled: bool,
}

pub fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::Relaxed)
}

pub fn prepare() -> Result<()> {
    // SAFETY: called only by the standalone, single-threaded worker before
    // fork. All pointers are valid stack objects and handlers only store an
    // atomic flag. No allocator/IO/locking is used by a signal handler.
    unsafe {
        libc::signal(libc::SIGCHLD, libc::SIG_DFL);
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = interrupt as *const () as usize;
        libc::sigemptyset(&mut action.sa_mask);
        for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            if libc::sigaction(signal, &action, std::ptr::null_mut()) != 0 {
                return Err(io::Error::last_os_error())
                    .context("Cannot install worker cancellation handler");
            }
        }
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM, 0, 0, 0);
    }
    Ok(())
}

/// Keep adoption scoped to this one request. Successful service/package
/// daemons pass to PID 1 when this supervisor exits; they must never become
/// children of the persistent worker and be killed by an unrelated later task.
pub fn run(
    timeout: Duration,
    managed: bool,
    may_modify: bool,
    keep_services: bool,
    execute: impl FnOnce(&std::path::Path) -> ModuleResult,
) -> Result<Supervised> {
    let main_pid = unsafe { libc::getpid() };
    let mut descriptors = [0; 2];
    if unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(io::Error::last_os_error()).context("Cannot create supervision pipe");
    }
    let supervisor = unsafe { libc::fork() };
    if supervisor < 0 {
        unsafe {
            libc::close(descriptors[0]);
            libc::close(descriptors[1]);
        }
        return Err(io::Error::last_os_error()).context("Cannot start request supervisor");
    }
    if supervisor == 0 {
        unsafe {
            libc::close(descriptors[0]);
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM, 0, 0, 0);
            if libc::getppid() != main_pid {
                libc::_exit(125);
            }
        }
        let execution = run_core(timeout, managed, may_modify, keep_services, execute)
            .unwrap_or_else(|error| {
                cancelled(
                    may_modify,
                    &format!("Request supervision failed: {error:#}"),
                )
            });
        let mut output = unsafe { File::from_raw_fd(descriptors[1]) };
        let status = if write_frame(&mut output, &execution, MAX_RESPONSE_BYTES).is_ok() {
            0
        } else {
            125
        };
        std::process::exit(status);
    }
    unsafe {
        libc::close(descriptors[1]);
    }
    let mut input = unsafe { File::from_raw_fd(descriptors[0]) };
    let flags = unsafe { libc::fcntl(input.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe { libc::fcntl(input.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        unsafe {
            libc::kill(supervisor, libc::SIGTERM);
            libc::waitpid(supervisor, std::ptr::null_mut(), 0);
        }
        return Err(io::Error::last_os_error()).context("Cannot configure supervisor pipe");
    }
    let deadline = Instant::now() + timeout + Duration::from_secs(10);
    let mut bytes = Vec::new();
    let mut signalled = false;
    let result = (|| -> Result<Supervised> {
        loop {
            if interrupted() && !signalled {
                unsafe {
                    libc::kill(supervisor, libc::SIGTERM);
                }
                signalled = true;
            }
            let mut chunk = [0; 65536];
            match input.read(&mut chunk) {
                Ok(0) => return decode(&bytes, MAX_RESPONSE_BYTES),
                Ok(length) => {
                    if bytes.len() + length > MAX_RESPONSE_BYTES + 4 {
                        bail!("Supervision result exceeded bound");
                    }
                    bytes.extend_from_slice(&chunk[..length]);
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error).context("Cannot read supervision result"),
            }
            if Instant::now() >= deadline {
                bail!("Request supervisor did not confirm cancellation/cleanup");
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    })();
    if result.is_err() {
        unsafe {
            libc::kill(supervisor, libc::SIGTERM);
        }
    }
    // A complete frame/EOF means its writer has exited. On an abnormal pipe
    // failure do not block the worker forever; parent death also signals it.
    let flags = if result.is_ok() { 0 } else { libc::WNOHANG };
    unsafe {
        libc::waitpid(supervisor, std::ptr::null_mut(), flags);
    }
    result
}

fn run_core(
    timeout: Duration,
    managed: bool,
    may_modify: bool,
    keep_services: bool,
    execute: impl FnOnce(&std::path::Path) -> ModuleResult,
) -> Result<Supervised> {
    if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } != 0 {
        return Err(io::Error::last_os_error()).context("Cannot establish request subreaper");
    }
    let mut builder = tempfile::Builder::new();
    builder.prefix(".rustsible-request-");
    let executable = std::env::current_exe()?;
    // The bootstrap shell owns the enclosing private directory and removes it
    // even if the worker is killed before Rust can drop its request scratch.
    let scratch = if managed {
        builder.tempdir_in(
            executable
                .parent()
                .context("Worker executable has no parent")?,
        )?
    } else {
        builder.tempdir()?
    };
    let (staging_parent, staging_child) = UnixDatagram::pair()?;
    let mut staging = crate::staging::Registry::new();
    let parent = unsafe { libc::getpid() };
    let mut descriptors = [0; 2];
    // SAFETY: descriptors points to storage for exactly two descriptors.
    if unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(io::Error::last_os_error()).context("Cannot create worker result pipe");
    }
    // SAFETY: no threads have been created in this process. The child only
    // owns its copied request, private result pipe, and execution subtree.
    let child = unsafe { libc::fork() };
    if child < 0 {
        unsafe {
            libc::close(descriptors[0]);
            libc::close(descriptors[1]);
        }
        return Err(io::Error::last_os_error()).context("Cannot fork worker execution");
    }
    if child == 0 {
        unsafe {
            libc::close(descriptors[0]);
            libc::setpgid(0, 0);
            for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
                libc::signal(signal, libc::SIG_DFL);
            }
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0);
            if libc::getppid() != parent {
                libc::_exit(125);
            }
        }
        drop(staging_parent);
        crate::staging::configure(staging_child.as_raw_fd());
        // Module tools must not consume the control/cancellation channel.
        let null = File::open("/dev/null");
        if let Ok(null) = null {
            unsafe {
                libc::dup2(null.as_raw_fd(), 0);
            }
        }
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| execute(scratch.path())))
                .unwrap_or_else(|_| ModuleResult {
                    failed: true,
                    uncertain: may_modify,
                    msg: "Native module panicked; result cannot be confirmed".into(),
                    ..ModuleResult::default()
                });
        let mut output = unsafe { File::from_raw_fd(descriptors[1]) };
        let status = if write_frame(&mut output, &Response::new(result), MAX_RESPONSE_BYTES).is_ok()
        {
            0
        } else {
            125
        };
        // Fork happens before threads; C atexit handlers are safe here and
        // write the child's coverage profile. Rust destructors are not rerun.
        std::process::exit(status);
    }
    unsafe {
        libc::close(descriptors[1]);
        libc::setpgid(child, child);
    }
    drop(staging_child);
    let mut pipe = unsafe { File::from_raw_fd(descriptors[0]) };
    let flags = unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        let error = io::Error::last_os_error();
        let _ = terminate_tree(child);
        return Err(error).context("Cannot configure result pipe");
    }
    let deadline = Instant::now()
        .checked_add(timeout)
        .context("Worker timeout is too large")?;
    let mut bytes = Vec::new();
    let result = (|| -> Result<Supervised> {
        loop {
            if INTERRUPTED.load(Ordering::Relaxed) || unsafe { libc::getppid() } == 1 {
                return Ok(cancelled(
                    may_modify,
                    "Worker cancelled by signal or parent exit",
                ));
            }
            if Instant::now() >= deadline {
                return Ok(cancelled(may_modify, "Worker execution timed out"));
            }
            let mut poll = [
                libc::pollfd {
                    fd: pipe.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: staging_parent.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: 0,
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            let status = unsafe { libc::poll(poll.as_mut_ptr(), poll.len() as libc::nfds_t, 20) };
            if status < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error).context("Cannot monitor worker execution");
            }
            if poll[1].revents & libc::POLLIN != 0 {
                staging.serve(&staging_parent)?;
            }
            if poll[0].revents != 0 {
                let mut chunk = [0; 64 * 1024];
                loop {
                    match pipe.read(&mut chunk) {
                        Ok(0) => {
                            let response: Response = decode(&bytes, MAX_RESPONSE_BYTES)?;
                            return Ok(Supervised {
                                result: response.validate()?,
                                cancelled: false,
                            });
                        }
                        Ok(length) => {
                            if bytes.len() + length > MAX_RESPONSE_BYTES + 4 {
                                bail!("Worker result exceeds its size bound");
                            }
                            bytes.extend_from_slice(&chunk[..length]);
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        Err(error) => return Err(error).context("Cannot read worker result"),
                    }
                }
            }
            if poll[2].revents != 0 {
                return Ok(cancelled(
                    may_modify,
                    "Worker cancelled: control channel closed or received unexpected input",
                ));
            }
        }
    })();
    // Cleanup is mandatory after both success and failure: synchronous shell
    // tools may otherwise leave background descendants even after returning.
    let retain = keep_services
        && result.as_ref().is_ok_and(|execution| {
            !execution.cancelled
                && !execution.result.uncertain
                && (!execution.result.failed || execution.result.changed)
        });
    if retain {
        // Only normal completed OS operations may transfer daemon ownership.
        // Cancelled/unconfirmed operations still terminate their entire tree.
        unsafe {
            libc::waitpid(child, std::ptr::null_mut(), 0);
            while libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG) > 0 {}
        }
    } else {
        terminate_tree(child)?;
    }
    staging.cleanup()?;
    result
}
fn cancelled(may_modify: bool, message: &str) -> Supervised {
    Supervised {
        cancelled: true,
        result: ModuleResult {
            failed: true,
            uncertain: may_modify,
            msg: message.into(),
            ..ModuleResult::default()
        },
    }
}
fn terminate_tree(group: libc::pid_t) -> Result<()> {
    unsafe {
        libc::kill(-group, libc::SIGKILL);
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let mut status = 0;
        let reaped = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
        if reaped > 0 {
            continue;
        }
        if reaped < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ECHILD) {
                return Ok(());
            }
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error).context("Cannot reap worker descendants");
        }
        // SIGCHLD is not ignored and this single-threaded parent is the sole
        // reaper: listed child PIDs cannot be reused before we wait on them.
        let children =
            fs::read_to_string(format!("/proc/self/task/{}/children", std::process::id()))
                .context("Cannot inspect adopted descendants")?;
        for child in children.split_whitespace() {
            let child = child.parse::<libc::pid_t>().context("Invalid child PID")?;
            if child > 0 {
                unsafe {
                    libc::kill(child, libc::SIGKILL);
                }
            }
        }
        if Instant::now() >= deadline {
            bail!("Worker descendants could not be reaped");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
