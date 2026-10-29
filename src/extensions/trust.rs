//! Installation authorization lives outside package contents. A repository
//! cannot self-authorize by shipping a forged in-package receipt. The lock
//! covers both directory publication and the authorization ledger update.
use super::*;
use std::os::fd::AsRawFd;
type Records = BTreeMap<String, BTreeMap<String, String>>;
pub(crate) struct Trust {
    _lock: Option<File>,
    pub records: Records,
    path: Option<PathBuf>,
}
fn directory() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".rustsible"))
}
impl Trust {
    pub fn read() -> Result<Self> {
        Self::open(false)
    }
    pub fn write(root: &Path) -> Result<Self> {
        let default =
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".rustsible/collections"));
        if default.as_ref().and_then(|path| path.canonicalize().ok()) != root.canonicalize().ok() {
            return Ok(Self {
                _lock: None,
                records: BTreeMap::new(),
                path: None,
            });
        }
        Self::open(true)
    }
    fn open(write: bool) -> Result<Self> {
        let Some(directory) = directory() else {
            return Ok(Self {
                _lock: None,
                records: BTreeMap::new(),
                path: None,
            });
        };
        let path = directory.join("native-trust.json");
        if !write && !path.exists() {
            return Ok(Self {
                _lock: None,
                records: BTreeMap::new(),
                path: Some(path),
            });
        }
        if write {
            rustsible_protocol::change::ensure_mutation_allowed()?;
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&directory)?;
        }
        let metadata = fs::symlink_metadata(&directory)?;
        if !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o022 != 0
        {
            bail!("Unsafe native trust directory");
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(write)
            .create(write)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(directory.join("native-trust.lock"))?;
        let metadata = lock.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1
        {
            bail!("Unsafe native trust lock");
        }
        if unsafe {
            libc::flock(
                lock.as_raw_fd(),
                if write { libc::LOCK_EX } else { libc::LOCK_SH },
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        let records = if path.exists() {
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.is_file()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.mode() & 0o022 != 0
            {
                bail!("Unsafe native authorization ledger");
            }
            read_json(&path)?
        } else {
            BTreeMap::new()
        };
        Ok(Self {
            _lock: Some(lock),
            records,
            path: Some(path),
        })
    }
    pub fn contains(&self, root: &Path, id: &str) -> Option<&str> {
        let root = root.canonicalize().ok()?;
        self.records
            .get(root.to_str()?)?
            .get(id)
            .map(String::as_str)
    }
    pub fn authorize(&mut self, root: &Path, id: &str, digest: &str) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let root = root.canonicalize()?;
        let root = root.to_str().context("Collection roots must be UTF-8")?;
        self.records
            .entry(root.into())
            .or_default()
            .insert(id.into(), digest.into());
        let bytes = serde_json::to_vec_pretty(&self.records)?;
        if bytes.len() > 1024 * 1024 {
            bail!("Native trust ledger exceeds 1 MiB");
        }
        crate::atomic::write(path, &bytes)
    }
}
