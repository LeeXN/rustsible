//! The supervisor creates and owns destination-side staging. A killed module
//! cannot run destructors; SCM_RIGHTS keeps its chosen directory inode pinned in
//! the parent so cleanup neither leaks files nor follows a renamed ancestor.
use anyhow::{bail, Context, Result};
use std::{
    cell::Cell,
    fs::File,
    io,
    os::{
        fd::{AsRawFd, FromRawFd, RawFd},
        unix::{fs::PermissionsExt, net::UnixDatagram},
    },
    path::PathBuf,
};
thread_local! { static CHANNEL: Cell<RawFd> = const { Cell::new(-1) }; }
pub fn configure(fd: RawFd) {
    CHANNEL.set(fd);
}

pub struct Stage {
    path: PathBuf,
}
impl Stage {
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}
pub fn create(directory: &File) -> Result<Stage> {
    rustsible_protocol::change::ensure_mutation_allowed()?;
    let socket = CHANNEL.get();
    if socket < 0 {
        bail!("Destination staging requires a supervisor");
    }
    send_descriptor(socket, directory.as_raw_fd())?;
    let mut reply = [0u8; 512];
    let length = unsafe { libc::recv(socket, reply.as_mut_ptr().cast(), reply.len(), 0) };
    if length <= 0 {
        return Err(io::Error::last_os_error())
            .context("Supervisor did not create destination staging");
    }
    let name = std::str::from_utf8(&reply[..length as usize]).context("Invalid staging reply")?;
    if !name
        .strip_prefix(".rustsible-write-")
        .is_some_and(|suffix| {
            !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
    {
        bail!("Supervisor could not create private destination staging");
    }
    Ok(Stage {
        path: PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd())).join(name),
    })
}
struct OwnedStage {
    // Field order matters: the temporary path uses this process's parent fd.
    directory: tempfile::TempDir,
    _parent: File,
}
#[derive(Default)]
pub struct Registry {
    stages: Vec<OwnedStage>,
}
impl Registry {
    pub fn new() -> Self {
        Self { stages: Vec::new() }
    }
    pub fn serve(&mut self, socket: &UnixDatagram) -> Result<()> {
        let parent = receive_descriptor(socket.as_raw_fd())?;
        let created = (|| -> Result<String> {
            crate::protection::ensure_safe_directory(&parent)?;
            let directory = tempfile::Builder::new()
                .prefix(".rustsible-write-")
                .permissions(std::fs::Permissions::from_mode(0o700))
                .tempdir_in(format!("/proc/self/fd/{}", parent.as_raw_fd()))?;
            let name = directory
                .path()
                .file_name()
                .and_then(|name| name.to_str())
                .context("Invalid generated staging name")?
                .to_owned();
            self.stages.push(OwnedStage {
                directory,
                _parent: parent,
            });
            Ok(name)
        })();
        let reply = created.unwrap_or_else(|_| "!staging allocation failed".into());
        match socket.send(reply.as_bytes()) {
            Ok(length) if length == reply.len() => Ok(()),
            Err(error)
                if matches!(error.raw_os_error(), Some(libc::ECONNREFUSED | libc::EPIPE)) =>
            {
                Ok(())
            }
            Err(error) => Err(error).context("Cannot acknowledge destination staging"),
            _ => bail!("Incomplete destination staging acknowledgement"),
        }
    }
    pub fn cleanup(mut self) -> Result<()> {
        let mut error = None;
        for stage in self.stages.drain(..) {
            if let Err(cause) = stage.directory.close() {
                error = Some(cause);
            }
        }
        if let Some(error) = error {
            return Err(error).context("Cannot clean destination staging");
        }
        Ok(())
    }
}
fn send_descriptor(socket: RawFd, fd: RawFd) -> Result<()> {
    let mut byte = [1u8];
    let mut iov = libc::iovec {
        iov_base: byte.as_mut_ptr().cast(),
        iov_len: 1,
    };
    // usize alignment is required by cmsghdr on both supported 64-bit ABIs.
    let mut control = [0usize; 8];
    unsafe {
        let mut message: libc::msghdr = std::mem::zeroed();
        message.msg_iov = &mut iov;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) as _;
        let header = libc::CMSG_FIRSTHDR(&message);
        (*header).cmsg_level = libc::SOL_SOCKET;
        (*header).cmsg_type = libc::SCM_RIGHTS;
        (*header).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) as _;
        std::ptr::write(libc::CMSG_DATA(header).cast::<RawFd>(), fd);
        if libc::sendmsg(socket, &message, libc::MSG_NOSIGNAL) != 1 {
            return Err(io::Error::last_os_error())
                .context("Cannot send staging directory descriptor");
        }
    }
    Ok(())
}
fn receive_descriptor(socket: RawFd) -> Result<File> {
    let mut bytes = [0u8; 16];
    let mut iov = libc::iovec {
        iov_base: bytes.as_mut_ptr().cast(),
        iov_len: bytes.len(),
    };
    let mut control = [0usize; 8];
    unsafe {
        let mut message: libc::msghdr = std::mem::zeroed();
        message.msg_iov = &mut iov;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = std::mem::size_of_val(&control) as _;
        let received = libc::recvmsg(socket, &mut message, libc::MSG_CMSG_CLOEXEC);
        if received < 0 {
            return Err(io::Error::last_os_error())
                .context("Cannot receive staging directory descriptor");
        }
        let mut descriptors = Vec::new();
        let mut header = libc::CMSG_FIRSTHDR(&message);
        while !header.is_null() {
            if (*header).cmsg_level == libc::SOL_SOCKET && (*header).cmsg_type == libc::SCM_RIGHTS {
                // musl exposes 32-bit cmsg lengths, glibc uses size_t.
                let size = ((*header).cmsg_len as u64).saturating_sub(libc::CMSG_LEN(0) as u64);
                for offset in 0..size / std::mem::size_of::<RawFd>() as u64 {
                    descriptors.push(File::from_raw_fd(std::ptr::read_unaligned(
                        libc::CMSG_DATA(header).cast::<RawFd>().add(offset as usize),
                    )));
                }
            }
            header = libc::CMSG_NXTHDR(&message, header);
        }
        if received != 1
            || bytes[0] != 1
            || message.msg_flags & (libc::MSG_TRUNC | libc::MSG_CTRUNC) != 0
            || descriptors.len() != 1
        {
            bail!("Invalid destination staging request");
        }
        descriptors.pop().context("Missing directory descriptor")
    }
}
