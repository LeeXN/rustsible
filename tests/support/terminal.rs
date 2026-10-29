//! A real controlling PTY verifies prompt echo/restoration without placing
//! fixture passwords in command arguments or allowing an interactive hang.
use anyhow::{bail, Context, Result};
use std::{
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::process::CommandExt,
    },
    process::{Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};
pub struct TerminalOutput {
    pub status: ExitStatus,
    pub transcript: String,
    pub echo_restored: bool,
}
pub fn run(command: &mut Command, passwords: &[&[u8]]) -> Result<TerminalOutput> {
    if passwords.iter().any(|password| password.is_empty()) {
        bail!("Fixture passwords must be non-empty");
    }
    let mut master = -1;
    let mut slave = -1;
    if unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut master = unsafe { File::from_raw_fd(master) };
    let slave = unsafe { File::from_raw_fd(slave) };
    for file in [&master, &slave] {
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    let fd = slave.as_raw_fd();
    command
        .stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave.try_clone()?));
    // The new session owns only this test's terminal and process group.
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() < 0 || libc::ioctl(fd, libc::TIOCSCTTY, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    let result = (|| -> Result<TerminalOutput> {
        let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
        if flags < 0
            || unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }
                < 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut output = Vec::new();
        let mut cursor = 0;
        let mut sent = 0;
        let mut exited = None;
        loop {
            let mut buffer = [0u8; 8192];
            loop {
                match master.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        output.extend_from_slice(&buffer[..count]);
                        if output.len() > 4 * 1024 * 1024 {
                            bail!("Terminal fixture output exceeded its bound");
                        }
                    }
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            || error.raw_os_error() == Some(libc::EIO) =>
                    {
                        break
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            while sent < passwords.len() {
                let Some(offset) = output[cursor..]
                    .windows(b"password: ".len())
                    .position(|window| window == b"password: ")
                else {
                    break;
                };
                let mut terminal = unsafe { std::mem::zeroed::<libc::termios>() };
                if unsafe { libc::tcgetattr(slave.as_raw_fd(), &mut terminal) } != 0 {
                    bail!("Cannot inspect prompt echo state");
                }
                if terminal.c_lflag & libc::ECHO != 0 {
                    break;
                }
                master.write_all(passwords[sent])?;
                if passwords[sent] != b"\x03" {
                    master.write_all(b"\n")?;
                }
                cursor += offset + b"password: ".len();
                sent += 1;
            }
            if let Some(status) = exited {
                let mut terminal = unsafe { std::mem::zeroed::<libc::termios>() };
                let echo_restored = unsafe { libc::tcgetattr(slave.as_raw_fd(), &mut terminal) }
                    == 0
                    && terminal.c_lflag & libc::ECHO != 0;
                if sent != passwords.len() {
                    bail!("Expected password prompts did not occur");
                }
                let transcript = String::from_utf8_lossy(&output).into_owned();
                for password in passwords.iter().filter(|password| **password != b"\x03") {
                    if transcript
                        .as_bytes()
                        .windows(password.len())
                        .any(|window| window == *password)
                    {
                        bail!("Password was echoed by terminal fixture");
                    }
                }
                return Ok(TerminalOutput {
                    status,
                    transcript,
                    echo_restored,
                });
            }
            exited = child.try_wait()?;
            if Instant::now() >= deadline {
                bail!("Password terminal fixture timed out");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    })();
    if result.is_err() {
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
        let _ = child.kill();
        let _ = child.wait();
    }
    result.context("Controlling-terminal acceptance failed")
}
