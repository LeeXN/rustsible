//! Native file updates keep content and protection metadata off the live inode
//! until a same-filesystem rename. Directory descriptors pin every later path
//! operation even if a parent directory is renamed during preparation.
use anyhow::{bail, Context, Result};
use rustsible_protocol::{
    change,
    param::{get_optional_param, parse_mode, validate_identity},
};
use serde_yaml::Value;
use std::{
    ffi::CString,
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Seek, Write},
    os::{
        fd::AsRawFd,
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        },
    },
    path::{Path, PathBuf},
};

#[derive(Default, Clone, Copy)]
pub struct Attributes {
    pub mode: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
}
impl Attributes {
    pub fn parse(args: &Value) -> Result<Self> {
        Ok(Self {
            mode: get_optional_param::<String>(args, "mode")?
                .map(|mode| parse_mode(&mode))
                .transpose()?,
            uid: get_optional_param::<String>(args, "owner")?
                .map(|owner| identity(&owner, false))
                .transpose()?,
            gid: get_optional_param::<String>(args, "group")?
                .map(|group| identity(&group, true))
                .transpose()?,
        })
    }
    pub fn differs(self, metadata: &Metadata) -> bool {
        self.mode
            .is_some_and(|mode| mode != metadata.mode() & 0o7777)
            || self.uid.is_some_and(|uid| uid != metadata.uid())
            || self.gid.is_some_and(|gid| gid != metadata.gid())
    }
    pub fn apply(self, file: &File) -> Result<()> {
        change::ensure_mutation_allowed()?;
        let old = file.metadata()?;
        if self.uid.is_some_and(|uid| uid != old.uid())
            || self.gid.is_some_and(|gid| gid != old.gid())
        {
            if unsafe {
                libc::fchownat(
                    file.as_raw_fd(),
                    c"".as_ptr(),
                    self.uid.unwrap_or(u32::MAX),
                    self.gid.unwrap_or(u32::MAX),
                    libc::AT_EMPTY_PATH,
                )
            } != 0
            {
                return Err(io::Error::last_os_error()).context("Cannot set file ownership");
            }
            change::record_change();
        }
        if let Some(mode) = self.mode {
            if file.metadata()?.mode() & 0o7777 != mode {
                fs::set_permissions(
                    format!("/proc/self/fd/{}", file.as_raw_fd()),
                    fs::Permissions::from_mode(mode),
                )?;
                change::record_change();
            }
            if file.metadata()?.mode() & 0o7777 != mode {
                bail!("Filesystem did not retain the requested file mode");
            }
        }
        Ok(())
    }
}
pub fn identity(value: &str, group: bool) -> Result<u32> {
    validate_identity(value)?;
    if value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Ok(value.parse()?);
    }
    let name = CString::new(value)?;
    let mut buffer = vec![0u8; 16384];
    loop {
        // Reentrant libc account lookup avoids shell parsing and keeps
        // numeric IDs and native account-database results distinct.
        let (code, id) = unsafe {
            if group {
                let mut entry: libc::group = std::mem::zeroed();
                let mut result = std::ptr::null_mut();
                let code = libc::getgrnam_r(
                    name.as_ptr(),
                    &mut entry,
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &mut result,
                );
                (
                    code,
                    (code == 0 && !result.is_null()).then_some(entry.gr_gid),
                )
            } else {
                let mut entry: libc::passwd = std::mem::zeroed();
                let mut result = std::ptr::null_mut();
                let code = libc::getpwnam_r(
                    name.as_ptr(),
                    &mut entry,
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &mut result,
                );
                (
                    code,
                    (code == 0 && !result.is_null()).then_some(entry.pw_uid),
                )
            }
        };
        if code == libc::ERANGE && buffer.len() < 1024 * 1024 {
            buffer.resize(buffer.len() * 2, 0);
            continue;
        }
        if code != 0 {
            return Err(io::Error::from_raw_os_error(code)).context("Cannot resolve file identity");
        }
        return id.context("Requested file owner/group does not exist");
    }
}

pub struct Location {
    pub directory: File,
    pub name: std::ffi::OsString,
}
impl Location {
    pub fn open(path: &str) -> Result<Self> {
        if path.is_empty()
            || path.contains('\0')
            || path.ends_with('/')
            || path.ends_with("/.")
            || path.ends_with("/..")
        {
            bail!("Destination must identify a filename");
        }
        let path = Path::new(path);
        let name = path
            .file_name()
            .filter(|name| *name != "." && *name != "..")
            .context("Destination needs a filename")?
            .to_os_string();
        let parent = path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_PATH)
            .open(parent)
            .context("Cannot open destination directory")?;
        Ok(Self { directory, name })
    }
    pub fn parent(&self) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}", self.directory.as_raw_fd()))
    }
    pub fn sync(&self) -> Result<()> {
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY)
            .open(self.parent())?
            .sync_all()
            .context("Cannot sync destination directory")
    }
    pub fn path(&self) -> PathBuf {
        self.parent().join(&self.name)
    }
    pub fn metadata(&self) -> Result<Option<Metadata>> {
        match fs::symlink_metadata(self.path()) {
            Ok(metadata) => Ok(Some(metadata)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).context("Cannot inspect destination"),
        }
    }
    pub fn ensure_safe_parent(&self) -> Result<()> {
        rustsible_native::protection::ensure_safe_directory(&self.directory)
    }
    pub fn pin_object(&self, expected: &Metadata) -> Result<File> {
        // Metadata operations need ownership, not read permission (mode=0000
        // must still be repairable by its owner). O_PATH pins that inode.
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_NOFOLLOW)
            .open(self.path())?;
        if signature(&file.metadata()?) != signature(expected) {
            bail!("Destination changed during inspection");
        }
        Ok(file)
    }
    pub fn open_object(&self, expected: &Metadata) -> Result<File> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(self.path())?;
        if signature(&file.metadata()?) != signature(expected) {
            bail!("Destination changed during inspection");
        }
        Ok(file)
    }
}
#[derive(PartialEq, Eq)]
struct Signature(u64, u64, u32, u32, u32, u64, i64, i64, i64, i64);
fn signature(metadata: &Metadata) -> Signature {
    Signature(
        metadata.dev(),
        metadata.ino(),
        metadata.mode(),
        metadata.uid(),
        metadata.gid(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    )
}

pub struct Destination {
    pub location: Location,
    pub original: Option<Metadata>,
    pub file: Option<File>,
    default_mode: u32,
}
impl Destination {
    pub fn open(path: &str) -> Result<Self> {
        let location = Location::open(path)?;
        let original = location.metadata()?;
        if original
            .as_ref()
            .is_some_and(|metadata| !metadata.is_file())
        {
            bail!("Destination must be a regular file, not a symbolic link or directory");
        }
        let file = original
            .as_ref()
            .map(|metadata| location.open_object(metadata))
            .transpose()?;
        Ok(Self {
            location,
            original,
            file,
            default_mode: 0o666 & !creation_umask(),
        })
    }
    pub fn unchanged(&self) -> Result<()> {
        let current = self.location.metadata()?;
        if current.as_ref().map(signature) != self.original.as_ref().map(signature)
            || self
                .file
                .as_ref()
                .map(|file| file.metadata().map(|metadata| signature(&metadata)))
                .transpose()?
                != self.original.as_ref().map(signature)
        {
            bail!("Destination changed during preparation");
        }
        Ok(())
    }
    pub fn equals(&mut self, source: &mut File) -> Result<bool> {
        let Some(existing) = self.file.as_mut() else {
            return Ok(false);
        };
        if existing.metadata()?.len() != source.metadata()?.len() {
            return Ok(false);
        }
        existing.rewind()?;
        source.rewind()?;
        Ok(crate::modules::hash(existing, "sha256")? == crate::modules::hash(source, "sha256")?)
    }
    pub fn set_metadata(&self, attributes: Attributes) -> Result<()> {
        self.unchanged()?;
        attributes.apply(
            self.file
                .as_ref()
                .context("Missing destination for metadata update")?,
        )
    }
    pub fn prepare(&self) -> Result<Prepared> {
        change::ensure_mutation_allowed()?;
        self.location.ensure_safe_parent()?;
        self.unchanged()?;
        self.location.sync()?;
        let directory = crate::staging::create(&self.location.directory)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(directory.path().join("content"))?;
        Ok(Prepared { directory, file })
    }
    pub fn install(&self, source: &mut impl Read, attributes: Attributes) -> Result<()> {
        let mut prepared = self.prepare()?;
        io::copy(source, &mut prepared.file)
            .context("Cannot prepare complete replacement content")?;
        self.commit(prepared, attributes)
    }
    pub fn commit(&self, mut prepared: Prepared, attributes: Attributes) -> Result<()> {
        change::ensure_mutation_allowed()?;
        let temporary = prepared.directory.path().join("content");
        let output = &mut prepared.file;
        preserve_metadata(output, self.file.as_ref(), attributes, self.default_mode)?;
        output.flush()?;
        output.sync_all()?;
        self.unchanged()?;
        if self.original.is_none() {
            rename_no_replace(&temporary, &self.location.path())?;
        } else {
            fs::rename(temporary, self.location.path()).context("Atomic replacement failed")?;
        }
        change::record_change();
        self.location
            .sync()
            .context("Replacement installed but destination directory sync failed")?;
        Ok(())
    }
}
pub struct Prepared {
    directory: crate::staging::Stage,
    pub file: File,
}

/// Restore ownership before capabilities/ACLs/mode so chown cannot clear a
/// protection that was just restored. Backups share the same exact policy.
pub fn preserve_metadata(
    output: &mut File,
    source: Option<&File>,
    attributes: Attributes,
    default_mode: u32,
) -> Result<()> {
    rustsible_native::protection::preserve(
        output,
        source,
        rustsible_native::protection::Protection {
            mode: attributes.mode,
            uid: attributes.uid,
            gid: attributes.gid,
        },
        default_mode,
    )
}

pub fn rename_no_replace(from: &Path, to: &Path) -> Result<()> {
    let from = CString::new(from.as_os_str().as_bytes())?;
    let to = CString::new(to.as_os_str().as_bytes())?;
    // Use the Linux syscall: libc's wrapper is not available on all musl ABIs.
    if unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    } != 0
    {
        return Err(io::Error::last_os_error())
            .context("Cannot atomically install without overwriting a concurrent entry");
    }
    Ok(())
}

pub fn touch(file: &File) -> Result<()> {
    change::ensure_mutation_allowed()?;
    let path = CString::new(format!("/proc/self/fd/{}", file.as_raw_fd()))?;
    if unsafe { libc::utimensat(libc::AT_FDCWD, path.as_ptr(), std::ptr::null(), 0) } != 0 {
        return Err(io::Error::last_os_error()).context("Cannot touch pinned path");
    }
    change::record_change();
    Ok(())
}

pub fn creation_umask() -> u32 {
    // Module children are isolated processes; capture before an HTTP client
    // creates threads. The worker's own sensitive files always specify 0600.
    unsafe {
        let mask = libc::umask(0);
        libc::umask(mask);
        mask
    }
}
