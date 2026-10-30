//! Archives are fully decoded and validated in private scratch before any
//! destination change. Archive links/devices and ambiguous duplicate names are
//! refused. Destination directory fds prevent archive-controlled symlink walks;
//! individual files reuse the atomic installer and protection-metadata policy.
use crate::filesystem::{Attributes, Destination, Location};
use anyhow::{bail, Context, Result};
use rustsible_native::archive::{decode, relative_path, Entry, MAX_TOTAL};
use rustsible_protocol::{change, registry::Builtin, ModuleResult, Request};
use serde_yaml::Value;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Seek},
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::{Component, Path, PathBuf},
};

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
