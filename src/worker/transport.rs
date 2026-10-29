use anyhow::{bail, Context, Result};
use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    os::fd::AsRawFd,
    process::{Child, ChildStderr, ChildStdin, ChildStdout},
    time::{Duration, Instant},
};

pub enum Session {
    Local {
        child: Child,
        input: Option<ChildStdin>,
        output: ChildStdout,
        diagnostic: ChildStderr,
        pending: VecDeque<u8>,
    },
    Remote {
        channel: ssh2::Channel,
        _session: ssh2::Session,
        pending: VecDeque<u8>,
    },
}
impl Session {
    pub fn local(command: &str) -> Result<Self> {
        let mut child = crate::ssh::connection::spawn_local_shell(command)?;
        let input = child.stdin.take().context("Missing worker input pipe")?;
        let output = child.stdout.take().context("Missing worker output pipe")?;
        let diagnostic = child
            .stderr
            .take()
            .context("Missing worker diagnostic pipe")?;
        for fd in [
            input.as_raw_fd(),
            output.as_raw_fd(),
            diagnostic.as_raw_fd(),
        ] {
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
            {
                let error = io::Error::last_os_error();
                let _ = child.kill();
                let _ = child.wait();
                return Err(error).context("Cannot configure worker pipes");
            }
        }
        Ok(Self::Local {
            child,
            input: Some(input),
            output,
            diagnostic,
            pending: VecDeque::new(),
        })
    }
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        match self {
            Self::Local { input, .. } => input
                .as_mut()
                .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "Worker input is closed"))?
                .write(data),
            Self::Remote { channel, .. } => channel.write(data),
        }
    }
    fn pending(&mut self) -> &mut VecDeque<u8> {
        match self {
            Self::Local { pending, .. } | Self::Remote { pending, .. } => pending,
        }
    }
    fn read_raw(&mut self, data: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Local { output, .. } => output.read(data),
            Self::Remote { channel, .. } => channel.read(data),
        }
    }
    fn read(&mut self, data: &mut [u8]) -> io::Result<usize> {
        let pending = self.pending();
        if !pending.is_empty() {
            let length = data.len().min(pending.len());
            for byte in &mut data[..length] {
                *byte = pending.pop_front().expect("Buffered worker byte");
            }
            return Ok(length);
        }
        self.read_raw(data)
    }
    fn prefetch(&mut self) -> Result<bool> {
        // A peer can report disk-full while the sender is still writing. Pump
        // stdout during upload so SSH windows cannot hide that structured error.
        let mut bytes = [0; 8192];
        match self.read_raw(&mut bytes) {
            Ok(0) => bail!("Worker closed its output while accepting input"),
            Ok(length) => {
                let pending = self.pending();
                if pending.len() + length > rustsible_protocol::MAX_RESPONSE_BYTES + 4 {
                    bail!("Worker response exceeds its bound");
                }
                pending.extend(&bytes[..length]);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error).context("Cannot pump worker upload response"),
        }
        let pending = self.pending();
        if pending.len() < 4 {
            return Ok(false);
        }
        let length = u32::from_be_bytes([pending[0], pending[1], pending[2], pending[3]]) as usize;
        if length == 0 || length > rustsible_protocol::MAX_RESPONSE_BYTES {
            bail!("Worker response exceeds its bound");
        }
        Ok(pending.len() >= length + 4)
    }
    pub fn diagnostics(&mut self, output: &mut Vec<u8>) -> Result<()> {
        let mut buffer = [0; 4096];
        loop {
            let read = match self {
                Self::Local { diagnostic, .. } => diagnostic.read(&mut buffer),
                Self::Remote { channel, .. } => channel.stderr().read(&mut buffer),
            };
            match read {
                Ok(0) => return Ok(()),
                Ok(length) => {
                    if output.len() + length > 64 * 1024 {
                        bail!("Worker diagnostics exceeded their bound");
                    }
                    output.extend_from_slice(&buffer[..length]);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error).context("Cannot read worker diagnostics"),
            }
        }
    }
    pub fn send(
        &mut self,
        data: &[u8],
        deadline: Instant,
        diagnostics: &mut Vec<u8>,
    ) -> Result<()> {
        self.send_inner(data, deadline, diagnostics, false)
    }
    pub fn send_payload(
        &mut self,
        data: &[u8],
        deadline: Instant,
        diagnostics: &mut Vec<u8>,
    ) -> Result<()> {
        self.send_inner(data, deadline, diagnostics, true)
    }
    fn send_inner(
        &mut self,
        mut data: &[u8],
        deadline: Instant,
        diagnostics: &mut Vec<u8>,
        payload: bool,
    ) -> Result<()> {
        while !data.is_empty() {
            self.diagnostics(diagnostics)?;
            if self.prefetch()? && payload {
                bail!("Worker returned a result before accepting the complete payload");
            }
            match self.write(data) {
                Ok(0) => bail!("Worker stopped accepting input"),
                Ok(length) => data = &data[length..],
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error).context("Cannot write worker input"),
            }
            if Instant::now() >= deadline {
                bail!("Worker input transfer timed out");
            }
            if !data.is_empty() {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        Ok(())
    }
    pub fn send_file(
        &mut self,
        file: &mut std::fs::File,
        deadline: Instant,
        diagnostics: &mut Vec<u8>,
    ) -> Result<()> {
        let mut buffer = [0; 64 * 1024];
        loop {
            let length = file.read(&mut buffer)?;
            if length == 0 {
                return Ok(());
            }
            self.send(&buffer[..length], deadline, diagnostics)?;
        }
    }
    fn receive_exact(
        &mut self,
        mut data: &mut [u8],
        deadline: Instant,
        diagnostics: &mut Vec<u8>,
    ) -> Result<()> {
        while !data.is_empty() {
            self.diagnostics(diagnostics)?;
            match self.read(data) {
                Ok(0) => {
                    self.diagnostics(diagnostics)?;
                    bail!("Worker closed its response stream");
                }
                Ok(length) => data = &mut data[length..],
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error).context("Cannot read worker response"),
            }
            if Instant::now() >= deadline {
                bail!("Worker response timed out");
            }
            if !data.is_empty() {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        Ok(())
    }
    pub fn receive<T: serde::de::DeserializeOwned>(
        &mut self,
        deadline: Instant,
        diagnostics: &mut Vec<u8>,
    ) -> Result<T> {
        let mut header = [0; 4];
        self.receive_exact(&mut header, deadline, diagnostics)?;
        let length = u32::from_be_bytes(header) as usize;
        if length == 0 || length > rustsible_protocol::MAX_RESPONSE_BYTES {
            bail!("Worker response exceeds its bound");
        }
        let mut frame = vec![0; length + 4];
        frame[..4].copy_from_slice(&header);
        self.receive_exact(&mut frame[4..], deadline, diagnostics)?;
        rustsible_protocol::decode(&frame, rustsible_protocol::MAX_RESPONSE_BYTES)
    }
    pub fn shutdown(&mut self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let closed = match self {
                Self::Local { input, .. } => {
                    input.take();
                    Ok(())
                }
                Self::Remote { channel, .. } => channel.send_eof().map_err(io::Error::from),
            };
            match closed {
                Ok(()) => break,
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(2))
                }
                Err(error) => return Err(error).context("Cannot cancel worker input"),
            }
        }
        let mut diagnostic = Vec::new();
        let mut buffer = [0; 8192];
        loop {
            self.diagnostics(&mut diagnostic)?;
            match self.read(&mut buffer) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error).context("Cannot drain worker during cleanup"),
            }
            if Instant::now() >= deadline {
                bail!("Worker did not finish cancellation/cleanup");
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        match self {
            Self::Local { child, .. } => loop {
                if let Some(status) = child.try_wait()? {
                    if !status.success() {
                        bail!("Worker cleanup exited unsuccessfully");
                    }
                    break;
                }
                if Instant::now() >= deadline {
                    bail!("Worker did not exit after cleanup");
                }
                std::thread::sleep(Duration::from_millis(2));
            },
            Self::Remote { channel, .. } => loop {
                match channel.wait_close() {
                    Ok(()) => {
                        if channel.exit_status()? != 0 {
                            bail!("Remote worker cleanup exited unsuccessfully");
                        }
                        break;
                    }
                    Err(error) => {
                        let error: io::Error = error.into();
                        if error.kind() != io::ErrorKind::WouldBlock {
                            return Err(error).context("Cannot close worker channel");
                        }
                    }
                }
                if Instant::now() >= deadline {
                    bail!("Worker channel did not close after cleanup");
                }
                std::thread::sleep(Duration::from_millis(2));
            },
        }
        Ok(())
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        // Normal lease teardown calls shutdown explicitly and reports errors.
        // This fallback still closes stdin after an interrupted bootstrap.
        match self {
            Self::Local { input, child, .. } => {
                input.take();
                let deadline = Instant::now() + Duration::from_secs(5);
                while child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
                if child.try_wait().ok().flatten().is_none() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
            Self::Remote { channel, .. } => {
                let _ = channel.send_eof();
                let _ = channel.close();
            }
        }
    }
}
