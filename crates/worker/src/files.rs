use crate::filesystem::{Attributes, Destination, Location};
use anyhow::{bail, Context, Result};
use rustsible_protocol::{
    change,
    param::{get_optional_param, get_param, validate_params},
    ModuleResult,
};
use serde_yaml::Value;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek},
    os::unix::{ffi::OsStrExt, fs::OpenOptionsExt},
    path::{Component, Path},
};

pub fn copy(args: &Value, payload: &Path, check: bool) -> Result<ModuleResult> {
    validate_params(args, &["dest", "mode", "owner", "group", "force"])?;
    let attributes = Attributes::parse(args)?;
    let dest = get_param::<String>(args, "dest")?;
    let force = get_optional_param::<bool>(args, "force")?.unwrap_or(true);
    let mut destination = Destination::open(&dest)?;
    let mut source = File::open(payload)?;
    let content_changed =
        destination.file.is_none() || (force && !destination.equals(&mut source)?);
    let changed = content_changed
        || destination
            .original
            .as_ref()
            .is_some_and(|metadata| attributes.differs(metadata));
    destination.unchanged()?;
    if changed {
        destination.location.ensure_safe_parent()?;
        if !check {
            if content_changed {
                source.rewind()?;
                destination.install(&mut source, attributes)?;
            } else {
                destination.set_metadata(attributes)?;
            }
        }
    }
    Ok(result(changed, "Content installed or already current"))
}
pub fn file(args: &Value, check: bool) -> Result<ModuleResult> {
    validate_params(
        args,
        &["path", "dest", "state", "src", "mode", "owner", "group"],
    )?;
    let attributes = Attributes::parse(args)?;
    if args.get("path").is_some() == args.get("dest").is_some() {
        bail!("File requires exactly one of path or dest");
    }
    let mut path = get_optional_param::<String>(args, "path")?
        .or(get_optional_param::<String>(args, "dest")?)
        .context("File requires a path")?;
    let state = get_optional_param::<String>(args, "state")?
        .unwrap_or_else(|| "file".into())
        .to_ascii_lowercase();
    if !["file", "directory", "link", "touch", "absent"].contains(&state.as_str()) {
        bail!("Unsupported file state");
    }
    if state == "absent" {
        validate_removal(&path)?;
    }
    let source = if state == "link" {
        if attributes.mode.is_some() {
            bail!("Mode cannot be applied to a symbolic link");
        }
        Some(get_param::<String>(args, "src")?)
    } else {
        None
    };
    if state != "file" {
        while path.ends_with('/') || path.ends_with("/.") {
            if path == "/" {
                break;
            }
            if path.ends_with("/.") {
                path.truncate(path.len() - 2);
            } else {
                path.pop();
            }
            if path.is_empty() {
                path = "/".into();
            }
        }
        if (state == "directory" || state == "touch") && (path == "/" || path == ".") {
            let directory = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY)
                .open(&path)?;
            let metadata = directory.metadata()?;
            let changed = state == "touch" || attributes.differs(&metadata);
            if changed && !check {
                attributes.apply(&directory)?;
                if state == "touch" {
                    crate::filesystem::touch(&directory)?;
                }
            }
            return Ok(result(changed, "Directory metadata is current or updated"));
        }
    }
    // mkdir -p is the one operation whose parent may not exist yet. Walk with
    // directory descriptors, never let a newly-created ancestor redirect work.
    let location = if state == "directory" {
        match Location::open(&path) {
            Ok(location) => location,
            Err(error) if !Path::new(&path).parent().unwrap_or(Path::new(".")).exists() => {
                if check {
                    return Ok(result(true, "Directory would be created"));
                }
                let _ = error;
                create_directories(Path::new(&path), attributes)?;
                let location = Location::open(&path)?;
                let metadata = location
                    .metadata()?
                    .context("Directory vanished after creation")?;
                attributes.apply(&location.pin_object(&metadata)?)?;
                return Ok(result(true, "Directory created"));
            }
            Err(error) => return Err(error),
        }
    } else {
        match Location::open(&path) {
            Ok(location) => location,
            Err(error)
                if state == "absent"
                    && error.downcast_ref::<std::io::Error>().is_some_and(|error| {
                        matches!(
                            error.kind(),
                            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                        )
                    }) =>
            {
                return Ok(result(false, "Path is already absent"))
            }
            Err(error) => return Err(error),
        }
    };
    let current = location.metadata()?;
    match state.as_str() {
        "absent" => {
            if let Some(metadata) = &current {
                location.ensure_safe_parent()?;
                if !check {
                    change::ensure_mutation_allowed()?;
                    if metadata.is_dir() {
                        fs::remove_dir_all(location.path())?;
                    } else {
                        fs::remove_file(location.path())?;
                    }
                    change::record_change();
                }
            }
            Ok(result(current.is_some(), "Path removed or already absent"))
        }
        "link" => {
            let source = source.context("Missing link source")?;
            if source.is_empty() || source.contains('\0') {
                bail!("Link source must be a nonempty path without NUL");
            }
            if current
                .as_ref()
                .is_some_and(|metadata| !metadata.file_type().is_symlink())
            {
                bail!("Link destination already exists and is not a symbolic link");
            }
            let changed = current.is_none()
                || fs::read_link(location.path())?.as_os_str() != std::ffi::OsStr::new(&source)
                || current
                    .as_ref()
                    .is_some_and(|metadata| attributes.differs(metadata));
            if changed {
                location.ensure_safe_parent()?;
                if !check {
                    change::ensure_mutation_allowed()?;
                    let staging = crate::staging::create(&location.directory)?;
                    let link = staging.path().join("link");
                    std::os::unix::fs::symlink(source, &link)?;
                    let cpath = std::ffi::CString::new(link.as_os_str().as_bytes())?;
                    if unsafe {
                        libc::lchown(
                            cpath.as_ptr(),
                            attributes.uid.unwrap_or(u32::MAX),
                            attributes.gid.unwrap_or(u32::MAX),
                        )
                    } != 0
                    {
                        return Err(std::io::Error::last_os_error())
                            .context("Cannot set symbolic link ownership");
                    }
                    if current.is_none() {
                        crate::filesystem::rename_no_replace(&link, &location.path())?;
                    } else {
                        fs::rename(link, location.path())?;
                    }
                    change::record_change();
                }
            }
            Ok(result(
                changed,
                "Symbolic link installed or already current",
            ))
        }
        "file" | "touch" | "directory" => {
            if let Some(metadata) = &current {
                if (state == "directory" && !metadata.is_dir())
                    || (state == "file" && !metadata.is_file())
                    || (state == "touch" && !(metadata.is_file() || metadata.is_dir()))
                {
                    bail!("Existing path has an incompatible type");
                }
                let changed = state == "touch" || attributes.differs(metadata);
                if changed && !check {
                    let object = location.pin_object(metadata)?;
                    attributes.apply(&object)?;
                    if state == "touch" {
                        crate::filesystem::touch(&object)?;
                    }
                }
                return Ok(result(changed, "File metadata is current or updated"));
            }
            if state == "file" {
                bail!("state=file does not create missing files");
            }
            location.ensure_safe_parent()?;
            if !check {
                change::ensure_mutation_allowed()?;
                if state == "directory" {
                    fs::create_dir(location.path())?;
                    change::record_change();
                    let metadata = location.metadata()?.context("Directory vanished")?;
                    attributes.apply(&location.pin_object(&metadata)?)?;
                } else {
                    Destination::open(&path)?.install(&mut std::io::empty(), attributes)?;
                }
            }
            Ok(result(true, "Path created or would be created"))
        }
        _ => unreachable!(),
    }
}
fn validate_removal(path: &str) -> Result<()> {
    if path.trim().is_empty()
        || !Path::new(path)
            .components()
            .any(|component| matches!(component, Component::Normal(_)))
        || Path::new(path)
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        bail!("Refusing unsafe removal path");
    }
    Ok(())
}
fn create_directories(path: &Path, attributes: Attributes) -> Result<()> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        bail!("Directory creation does not accept parent traversal");
    }
    let mut directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_PATH)
        .open(if path.is_absolute() { "/" } else { "." })?;
    for component in path.components() {
        let Component::Normal(name) = component else {
            if component == Component::ParentDir {
                bail!("Directory creation does not accept parent traversal");
            }
            continue;
        };
        let location = Location {
            directory,
            name: name.into(),
        };
        if location.metadata()?.is_none() {
            location.ensure_safe_parent()?;
            change::ensure_mutation_allowed()?;
            fs::create_dir(location.path())?;
            change::record_change();
            let metadata = location.metadata()?.context("Created directory vanished")?;
            attributes.apply(&location.pin_object(&metadata)?)?;
        }
        directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_PATH)
            .open(location.path())
            .context("Directory creation encountered a symbolic link or non-directory")?;
    }
    Ok(())
}
pub fn lineinfile(args: &Value, check: bool) -> Result<ModuleResult> {
    validate_params(
        args,
        &[
            "path",
            "line",
            "regexp",
            "state",
            "backup",
            "create",
            "insertafter",
            "insertbefore",
            "owner",
            "group",
            "mode",
        ],
    )?;
    let attributes = Attributes::parse(args)?;
    let path = get_param::<String>(args, "path")?;
    let state = get_optional_param::<String>(args, "state")?.unwrap_or_else(|| "present".into());
    let create = get_optional_param::<bool>(args, "create")?.unwrap_or(false);
    let backup = get_optional_param::<bool>(args, "backup")?.unwrap_or(false);
    let line = get_optional_param::<String>(args, "line")?;
    let regexp = get_optional_param::<String>(args, "regexp")?;
    let after = get_optional_param::<String>(args, "insertafter")?;
    let before = get_optional_param::<String>(args, "insertbefore")?;
    if after.is_some() && before.is_some() {
        bail!("insertafter and insertbefore are mutually exclusive");
    }
    for expression in [&regexp, &after, &before].into_iter().flatten() {
        if !["EOF", "BOF"].contains(&expression.as_str()) {
            regex::Regex::new(expression)?;
        }
    }
    if state == "present" && line.is_none() {
        bail!("state=present requires line");
    }
    if state == "absent" && line.is_none() && regexp.is_none() {
        bail!("state=absent requires line or regexp");
    }
    if !["present", "absent"].contains(&state.as_str()) {
        bail!("Invalid lineinfile state");
    }
    let mut destination = Destination::open(&path)?;
    if destination.file.is_none() && !create {
        if state == "absent" {
            return Ok(result(false, "File is already absent"));
        }
        bail!("Missing lineinfile path requires create=true");
    }
    let mut content = String::new();
    if let Some(existing) = &mut destination.file {
        existing
            .take(64 * 1024 * 1024 + 1)
            .read_to_string(&mut content)?;
        if content.len() > 64 * 1024 * 1024 {
            bail!("lineinfile input exceeds 64 MiB");
        }
    }
    let content_changed = crate::lines::process_line_modifications(
        &mut content,
        line,
        regexp,
        &state,
        after,
        before,
    )?;
    let changed = content_changed
        || destination
            .original
            .as_ref()
            .is_some_and(|metadata| attributes.differs(metadata));
    let mut result = result(changed, "File lines are current or updated");
    if changed {
        destination.location.ensure_safe_parent()?;
        if !check {
            if backup && content_changed && destination.file.is_some() {
                let stage = crate::staging::create(&destination.location.directory)?;
                let backup_path = stage.path().join("backup");
                let mut backup = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&backup_path)?;
                let mut existing = destination
                    .file
                    .as_ref()
                    .context("Missing backup source")?
                    .try_clone()?;
                existing.rewind()?;
                std::io::copy(&mut existing, &mut backup)?;
                crate::filesystem::preserve_metadata(
                    &mut backup,
                    Some(&existing),
                    Attributes::default(),
                    0o600,
                )?;
                let metadata = existing.metadata()?;
                use std::os::{fd::AsRawFd, unix::fs::MetadataExt};
                let times = [
                    libc::timespec {
                        tv_sec: metadata.atime(),
                        tv_nsec: metadata.atime_nsec(),
                    },
                    libc::timespec {
                        tv_sec: metadata.mtime(),
                        tv_nsec: metadata.mtime_nsec(),
                    },
                ];
                if unsafe { libc::futimens(backup.as_raw_fd(), times.as_ptr()) } != 0 {
                    return Err(std::io::Error::last_os_error())
                        .context("Cannot preserve backup timestamps");
                }
                backup.sync_all()?;
                let nonce = stage
                    .path()
                    .file_name()
                    .and_then(|name| name.to_str())
                    .and_then(|name| name.strip_prefix(".rustsible-write-"))
                    .context("Invalid staging name")?;
                let name = format!(
                    "{}.{}.bak",
                    destination.location.name.to_string_lossy(),
                    nonce
                );
                let final_path = destination.location.parent().join(&name);
                destination.unchanged()?;
                crate::filesystem::rename_no_replace(&backup_path, &final_path)?;
                change::record_change();
                result.values.insert(
                    "backup_file".into(),
                    Value::String(
                        fs::read_link(destination.location.parent())?
                            .join(name)
                            .to_string_lossy()
                            .into_owned(),
                    ),
                );
            }
            if content_changed {
                destination.install(&mut content.as_bytes(), attributes)?;
            } else {
                destination.set_metadata(attributes)?;
            }
        }
    }
    Ok(result)
}
fn result(changed: bool, message: &str) -> ModuleResult {
    ModuleResult {
        changed,
        msg: message.into(),
        ..ModuleResult::default()
    }
}
