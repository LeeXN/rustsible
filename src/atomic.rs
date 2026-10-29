//! Controller-side atomic file replacement with pinned parent and protection
//! metadata preservation; target modules use their supervisor-owned staging.
use anyhow::{bail, Context, Result};
use std::{
    ffi::OsString,
    fs::{self, File, Metadata, OpenOptions},
    io::Write,
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};
pub(crate) struct Target {
    parent: File,
    name: OsString,
    pub(crate) original: Option<File>,
    signature: Option<Signature>,
}
type Signature = (u64, u64, u64, i64, i64, i64, i64, u64, u32, u32, u32);
fn signature(metadata: &Metadata) -> Signature {
    (
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec(),
        metadata.nlink(),
        metadata.mode(),
        metadata.uid(),
        metadata.gid(),
    )
}
impl Target {
    fn path(&self) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}", self.parent.as_raw_fd())).join(&self.name)
    }
    pub(crate) fn open(path: &Path) -> Result<Self> {
        if path.as_os_str().is_empty()
            || ["/", "/.", "/.."]
                .iter()
                .any(|suffix| path.to_string_lossy().ends_with(suffix))
        {
            bail!("File output requires a filename");
        }
        let name = path
            .file_name()
            .filter(|name| *name != "." && *name != "..")
            .context("File file needs a name")?
            .to_owned();
        let parent = path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_PATH)
            .open(parent)
            .context("Cannot open File parent directory")?;
        let mut target = Self {
            parent,
            name,
            original: None,
            signature: None,
        };
        match fs::symlink_metadata(target.path()) {
            Ok(metadata) => {
                if !metadata.is_file() {
                    bail!("File files must be regular files, not links or directories");
                }
                let file = OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                    .open(target.path())?;
                if signature(&file.metadata()?) != signature(&metadata) {
                    bail!("File file changed during inspection");
                }
                target.signature = Some(signature(&metadata));
                target.original = Some(file);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(target)
    }
    pub(crate) fn unchanged(&self) -> Result<()> {
        let current = match fs::symlink_metadata(self.path()) {
            Ok(metadata) => Some(signature(&metadata)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        if current != self.signature
            || self
                .original
                .as_ref()
                .map(|file| file.metadata().map(|metadata| signature(&metadata)))
                .transpose()?
                != self.signature
        {
            bail!("File file changed before replacement");
        }
        Ok(())
    }
    fn sync(&self) -> Result<()> {
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY)
            .open(format!("/proc/self/fd/{}", self.parent.as_raw_fd()))?
            .sync_all()
            .context("Cannot sync File parent directory")
    }
    pub(crate) fn write(&self, bytes: &[u8]) -> Result<()> {
        rustsible_protocol::change::ensure_mutation_allowed()?;
        let metadata = self.parent.metadata()?;
        if metadata.uid() != 0 && metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o022 != 0 && metadata.mode() & 0o1000 == 0
        {
            bail!("File output parent has unsafe ownership or write permissions");
        }
        self.unchanged()?;
        self.sync()?;
        let parent = PathBuf::from(format!("/proc/self/fd/{}", self.parent.as_raw_fd()));
        let mut temporary = tempfile::Builder::new()
            .prefix(".rustsible-vault-")
            .tempfile_in(parent)?;
        temporary.write_all(bytes)?;
        rustsible_native::protection::preserve(
            temporary.as_file_mut(),
            self.original.as_ref(),
            Default::default(),
            0o600,
        )?;
        temporary.as_file().sync_all()?;
        self.unchanged()?;
        if self.original.is_some() {
            temporary
                .persist(self.path())
                .map_err(|_| anyhow::anyhow!("Atomic File replacement failed"))?;
        } else {
            temporary
                .persist_noclobber(self.path())
                .map_err(|_| anyhow::anyhow!("File output appeared during preparation"))?;
        }
        rustsible_protocol::change::record_change();
        self.sync()
            .context("File replacement installed but directory sync failed")
    }
}

pub(crate) fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    Target::open(path)?.write(bytes)
}
