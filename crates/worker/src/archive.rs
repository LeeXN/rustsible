//! Archives are fully decoded and validated in private scratch before any
//! destination change. Archive links/devices and ambiguous duplicate names are
//! refused. Destination directory fds prevent archive-controlled symlink walks;
//! individual files reuse the atomic installer and protection-metadata policy.
use crate::filesystem::{Attributes, Destination, Location};
use anyhow::{bail, Context, Result};
use rustsible_protocol::{change, registry::Builtin, ModuleResult, Request};
use serde_yaml::Value;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek},
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::{Component, Path, PathBuf},
};
const MAX_TOTAL: u64 = 8 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;
struct Entry {
    staged: Option<PathBuf>,
    mode: u32,
}

pub fn execute(request: &Request, scratch: &Path, payload: Option<&Path>) -> Result<ModuleResult> {
    Builtin::Unarchive.validate_parameters(&request.args)?;
    let args = &request.args;
    let attributes = Attributes::parse(args)?;
    let root = directory(Path::new(
        args.get("dest")
            .and_then(Value::as_str)
            .context("Unarchive requires dest")?,
    ))?;
    safe_directory(&root)?;
    if let Some(creates) = args.get("creates").and_then(Value::as_str) {
        let dest = fs::canonicalize(fd_path(&root))?;
        let creates = Path::new(creates);
        let relative = if creates.is_absolute() {
            creates
                .strip_prefix(&dest)
                .context("Unarchive creates must be below dest")?
        } else {
            creates
        };
        let relative = relative_path(relative)?;
        if relative.as_os_str().is_empty() {
            bail!("Unarchive creates must identify an entry below dest");
        }
        if let Some(parent) = descend(&root, relative.parent().unwrap_or(Path::new("")))? {
            match fs::symlink_metadata(fd_path(&parent).join(relative.file_name().unwrap())) {
                Ok(_) => {
                    return Ok(ModuleResult {
                        msg: "Unarchive creates already exists".into(),
                        ..Default::default()
                    })
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error).context("Cannot inspect unarchive creates"),
            }
        }
    }
    let source = if args.get("remote_src").and_then(Value::as_bool) == Some(true) {
        Path::new(
            args.get("src")
                .and_then(Value::as_str)
                .context("Unarchive requires src")?,
        )
    } else {
        payload.context("Unarchive requires streamed source")?
    };
    let input = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(source)
        .context("Cannot open archive source")?;
    if !input.metadata()?.is_file() || input.metadata()?.len() > MAX_TOTAL {
        bail!("Archive source must be a regular file no larger than 8 GiB");
    }
    let temporary = tempfile::tempdir_in(scratch)?;
    let entries = decode(input, temporary.path())?;
    // Preflight every existing path and all metadata parameters before creating
    // directories or replacing a file. A later IO failure can leave earlier
    // complete files installed, but never a truncated replacement.
    let mut changed = false;
    for (relative, entry) in &entries {
        if let Some(parent) = descend(&root, relative.parent().unwrap_or(Path::new("")))? {
            safe_directory(&parent)?;
            let path = fd_path(&parent).join(relative.file_name().unwrap());
            match &entry.staged {
                Some(staged) => {
                    let mut destination =
                        Destination::open(path.to_str().context("Invalid archive path")?)?;
                    let effective =
                        file_attributes(attributes, entry, destination.original.is_some());
                    changed |= !destination.equals(&mut File::open(staged)?)?
                        || destination
                            .original
                            .as_ref()
                            .is_some_and(|metadata| effective.differs(metadata));
                }
                None => match fs::symlink_metadata(&path) {
                    Ok(metadata) if metadata.is_dir() => changed |= attributes.differs(&metadata),
                    Ok(_) => bail!("Archive directory conflicts with an existing file or link"),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => changed = true,
                    Err(error) => return Err(error.into()),
                },
            }
        } else {
            changed = true;
        }
    }
    if request.check || !changed {
        return Ok(result(changed, entries.len()));
    }
    let mut created_modes = BTreeMap::new();
    for (relative, entry) in entries.iter().filter(|(_, entry)| entry.staged.is_none()) {
        let parent = descend(&root, relative.parent().unwrap_or(Path::new("")))?
            .context("Missing archive parent")?;
        safe_directory(&parent)?;
        let path = fd_path(&parent).join(relative.file_name().unwrap());
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                change::ensure_mutation_allowed()?;
                std::os::unix::fs::DirBuilderExt::mode(&mut fs::DirBuilder::new(), 0o700)
                    .create(&path)?;
                change::record_change();
                // New directories receive archive mode only when no mode was
                // explicitly supplied. Existing directory modes stay intact.
                let file = directory(&path)?;
                let mut mode = attributes;
                mode.mode = Some(
                    attributes
                        .mode
                        .unwrap_or(entry.mode & !crate::filesystem::creation_umask()),
                );
                // Metadata is applied after children, including non-writable modes.
                created_modes.insert(relative.clone(), mode);
                drop(file);
            }
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => bail!("Archive directory changed into a file or link"),
            Err(error) => return Err(error.into()),
        }
    }
    for (relative, entry) in entries.iter().filter(|(_, entry)| entry.staged.is_some()) {
        let parent = descend(&root, relative.parent().unwrap_or(Path::new("")))?
            .context("Missing archive parent")?;
        let path = fd_path(&parent).join(relative.file_name().unwrap());
        let mut destination = Destination::open(path.to_str().context("Invalid archive path")?)?;
        let effective = file_attributes(attributes, entry, destination.original.is_some());
        let mut file = File::open(entry.staged.as_ref().unwrap())?;
        if !destination.equals(&mut file)? {
            file.rewind()?;
            destination.install(&mut file, effective)?;
        } else if destination
            .original
            .as_ref()
            .is_some_and(|metadata| effective.differs(metadata))
        {
            destination.set_metadata(effective)?;
        }
    }
    for (relative, _) in entries
        .iter()
        .rev()
        .filter(|(_, entry)| entry.staged.is_none())
    {
        let file = descend(&root, relative)?.context("Archive directory disappeared")?;
        let effective = created_modes.get(relative).copied().unwrap_or(attributes);
        effective.apply(&file)?;
    }
    root.sync_all()?;
    Ok(result(change::known_changed(), entries.len()))
}
fn file_attributes(mut attributes: Attributes, entry: &Entry, existing: bool) -> Attributes {
    if !existing && attributes.mode.is_none() {
        attributes.mode = Some(entry.mode & !crate::filesystem::creation_umask());
    }
    attributes
}
fn result(changed: bool, count: usize) -> ModuleResult {
    ModuleResult {
        changed,
        msg: "Archive entries are installed".into(),
        values: std::collections::HashMap::from([("entries".into(), Value::Number(count.into()))]),
        ..Default::default()
    }
}
fn fd_path(file: &File) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
}
fn directory(path: &Path) -> Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
        .context("Archive destination must be a real accessible directory")
}
fn safe_directory(file: &File) -> Result<()> {
    Location {
        directory: file.try_clone()?,
        name: "unused".into(),
    }
    .ensure_safe_parent()
}
fn descend(root: &File, path: &Path) -> Result<Option<File>> {
    let mut current = root.try_clone()?;
    for component in path.components() {
        safe_directory(&current)?;
        let Component::Normal(name) = component else {
            bail!("Unsafe archive path component");
        };
        match directory(&fd_path(&current).join(name)) {
            Ok(next) => current = next,
            Err(error)
                if error
                    .downcast_ref::<io::Error>()
                    .is_some_and(|error| error.kind() == io::ErrorKind::NotFound) =>
            {
                return Ok(None)
            }
            Err(error) => return Err(error),
        }
    }
    Ok(Some(current))
}
fn relative_path(path: &Path) -> Result<PathBuf> {
    let bytes = path.to_str().context("Archive paths must be UTF-8")?;
    if bytes.len() > 4096 || bytes.contains(['\\', ':', '\0']) {
        bail!("Unsafe or oversized archive path");
    }
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => output.push(name),
            Component::CurDir => {}
            _ => bail!("Archive path escapes destination"),
        }
    }
    Ok(output)
}
fn decode(mut input: File, scratch: &Path) -> Result<BTreeMap<PathBuf, Entry>> {
    let mut magic = [0u8; 4];
    let count = input.read(&mut magic)?;
    input.rewind()?;
    let mut entries = BTreeMap::new();
    let mut total = 0;
    if count >= 2 && magic[..2] == *b"PK" {
        let mut archive = zip::ZipArchive::new(input).context("Invalid ZIP archive")?;
        if archive.len() > MAX_ENTRIES {
            bail!("Too many archive entries");
        }
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).context("Cannot read ZIP entry")?;
            if entry.is_symlink()
                || entry.unix_mode().is_some_and(|mode| {
                    mode & libc::S_IFMT != 0
                        && !matches!(mode & libc::S_IFMT, libc::S_IFREG | libc::S_IFDIR)
                })
            {
                bail!("Archive links and special files are unsupported");
            }
            let path = relative_path(Path::new(entry.name()?.as_ref()))?;
            let directory = entry.is_dir();
            if directory && entry.size() != 0 {
                bail!("Archive directory contains file data");
            }
            let mode = entry
                .unix_mode()
                .unwrap_or(if directory { 0o755 } else { 0o644 })
                & 0o777;
            add(
                &mut entries,
                path,
                directory,
                mode,
                &mut entry,
                scratch,
                &mut total,
            )?;
        }
    } else {
        let decoded;
        let source = if count >= 2 && magic[..2] == [0x1f, 0x8b] {
            decoded = scratch.join("decoded.tar");
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&decoded)?;
            let bytes = io::copy(
                &mut flate2::read::MultiGzDecoder::new(input).take(MAX_TOTAL + 1),
                &mut file,
            )
            .context("Invalid or truncated gzip archive")?;
            if bytes > MAX_TOTAL {
                bail!("Expanded archive exceeds 8 GiB");
            }
            file.rewind()?;
            file
        } else {
            input
        };
        let mut archive = tar::Archive::new(source);
        for entry in archive.entries().context("Invalid tar archive")? {
            let mut entry = entry.context("Invalid or truncated tar entry")?;
            let kind = entry.header().entry_type();
            if !kind.is_file() && !kind.is_dir() {
                bail!("Archive links and special files are unsupported");
            }
            if kind.is_dir() && entry.size() != 0 {
                bail!("Archive directory contains file data");
            }
            let path = relative_path(&entry.path()?)?;
            let mode = entry.header().mode()? & 0o777;
            add(
                &mut entries,
                path,
                kind.is_dir(),
                mode,
                &mut entry,
                scratch,
                &mut total,
            )?;
        }
    }
    Ok(entries)
}
fn add(
    entries: &mut BTreeMap<PathBuf, Entry>,
    path: PathBuf,
    is_dir: bool,
    mode: u32,
    input: &mut impl Read,
    scratch: &Path,
    total: &mut u64,
) -> Result<()> {
    if path.as_os_str().is_empty() {
        if is_dir {
            return Ok(());
        }
        bail!("Archive file needs a name");
    }
    if entries.len() >= MAX_ENTRIES {
        bail!("Too many archive entries");
    }
    if let Some(previous) = entries.get_mut(&path) {
        if is_dir && previous.staged.is_none() {
            previous.mode = mode;
            return Ok(());
        }
        bail!("Duplicate archive file path");
    }
    let mut parent = path.parent();
    while let Some(ancestor) = parent.filter(|path| !path.as_os_str().is_empty()) {
        if entries
            .get(ancestor)
            .is_some_and(|entry| entry.staged.is_some())
        {
            bail!("Archive file used as a parent directory");
        }
        entries.entry(ancestor.to_owned()).or_insert(Entry {
            staged: None,
            mode: 0o755,
        });
        if entries.len() >= MAX_ENTRIES {
            bail!("Too many archive entries");
        }
        parent = ancestor.parent();
    }
    if !is_dir && entries.keys().any(|existing| existing.starts_with(&path)) {
        bail!("Archive file conflicts with a directory");
    }
    let staged = if is_dir {
        None
    } else {
        let file = scratch.join(format!("entry-{}", entries.len()));
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&file)?;
        let copied = io::copy(
            &mut input.take(MAX_TOTAL.saturating_sub(*total) + 1),
            &mut output,
        )
        .context("Invalid, truncated or corrupt archive content")?;
        *total = total.checked_add(copied).context("Archive size overflow")?;
        if *total > MAX_TOTAL {
            bail!("Expanded archive exceeds 8 GiB");
        }
        Some(file)
    };
    entries.insert(path, Entry { staged, mode });
    Ok(())
}
