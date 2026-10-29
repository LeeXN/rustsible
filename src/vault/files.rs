//! Vault rewrites use a pinned parent, private same-directory inode and the
//! target installer's protection-metadata policy. Failure before rename leaves
//! the old file intact; no plaintext temporary is created by encrypt/rekey.
use anyhow::{bail, Context, Result};
use std::{
    ffi::OsString,
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;
pub struct Input {
    pub bytes: Zeroizing<Vec<u8>>,
    original: Option<Target>,
}
struct Target {
    parent: File,
    name: OsString,
    original: Option<File>,
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
    fn open(path: &Path) -> Result<Self> {
        if path.as_os_str().is_empty()
            || ["/", "/.", "/.."]
                .iter()
                .any(|suffix| path.to_string_lossy().ends_with(suffix))
        {
            bail!("Vault output requires a filename");
        }
        let name = path
            .file_name()
            .filter(|name| *name != "." && *name != "..")
            .context("Vault file needs a name")?
            .to_owned();
        let parent = path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_PATH)
            .open(parent)
            .context("Cannot open Vault parent directory")?;
        let mut target = Self {
            parent,
            name,
            original: None,
            signature: None,
        };
        match fs::symlink_metadata(target.path()) {
            Ok(metadata) => {
                if !metadata.is_file() {
                    bail!("Vault files must be regular files, not links or directories");
                }
                let file = OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                    .open(target.path())?;
                if signature(&file.metadata()?) != signature(&metadata) {
                    bail!("Vault file changed during inspection");
                }
                target.signature = Some(signature(&metadata));
                target.original = Some(file);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(target)
    }
    fn unchanged(&self) -> Result<()> {
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
            bail!("Vault file changed before replacement");
        }
        Ok(())
    }
    fn sync(&self) -> Result<()> {
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY)
            .open(format!("/proc/self/fd/{}", self.parent.as_raw_fd()))?
            .sync_all()
            .context("Cannot sync Vault parent directory")
    }
    fn write(&self, bytes: &[u8]) -> Result<()> {
        rustsible_protocol::change::ensure_mutation_allowed()?;
        let metadata = self.parent.metadata()?;
        if metadata.uid() != 0 && metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o022 != 0 && metadata.mode() & 0o1000 == 0
        {
            bail!("Vault output parent has unsafe ownership or write permissions");
        }
        self.unchanged()?;
        self.sync()?;
        let parent = PathBuf::from(format!("/proc/self/fd/{}", self.parent.as_raw_fd()));
        let mut temporary = tempfile::Builder::new()
            .prefix(".rustsible-vault-")
            .tempfile_in(parent)?;
        temporary.write_all(bytes)?;
        rustsible_protocol::protection::preserve(
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
                .map_err(|_| anyhow::anyhow!("Atomic Vault replacement failed"))?;
        } else {
            temporary
                .persist_noclobber(self.path())
                .map_err(|_| anyhow::anyhow!("Vault output appeared during preparation"))?;
        }
        rustsible_protocol::change::record_change();
        self.sync()
            .context("Vault replacement installed but directory sync failed")
    }
}
impl Input {
    pub fn read(path: &Path) -> Result<Self> {
        let mut bytes = Zeroizing::new(Vec::new());
        let original = if path == Path::new("-") {
            std::io::stdin()
                .take(super::MAX_ENVELOPE as u64 + 1)
                .read_to_end(&mut bytes)?;
            None
        } else {
            let target = Target::open(path)?;
            target
                .original
                .as_ref()
                .context("Vault input file does not exist")?
                .take(super::MAX_ENVELOPE as u64 + 1)
                .read_to_end(&mut bytes)?;
            target.unchanged()?;
            Some(target)
        };
        if bytes.len() > super::MAX_ENVELOPE {
            bail!("Vault input exceeds 80 MiB");
        }
        Ok(Self { bytes, original })
    }
    pub fn write(&self, input: &Path, output: Option<&Path>, bytes: &[u8]) -> Result<()> {
        if let Some(original) = &self.original {
            original.unchanged()?;
        }
        let output = output.unwrap_or(input);
        if output == Path::new("-") {
            std::io::stdout().lock().write_all(bytes)?;
            return Ok(());
        }
        let target = Target::open(output)?;
        target.write(bytes)
    }
}
