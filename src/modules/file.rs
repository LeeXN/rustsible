use anyhow::Result;
use log::info;
use serde_yaml::Value;
use std::path::Component;

use crate::inventory::Host;
use crate::modules::param::{get_optional_param, get_param, validate_params};
use crate::modules::ModuleResult;
use crate::ssh::connection::{Connection, SshConnection};

/// Quote one argument for a POSIX shell command.
///
/// Remote commands are currently transported as shell strings. Keeping the
/// quoting in one place prevents playbook and inventory values from becoming
/// additional shell syntax.
pub(crate) fn quote_posix_shell_arg(value: &str) -> Result<String> {
    if value.contains('\0') {
        return Err(anyhow::anyhow!("Shell arguments cannot contain NUL bytes"));
    }

    Ok(format!("'{}'", value.replace('\'', "'\"'\"'")))
}

fn validate_removal_path(path: &str) -> Result<()> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!("Refusing to remove an empty path"));
    }

    let mut normal_components = 0usize;
    for component in std::path::Path::new(trimmed).components() {
        match component {
            Component::Normal(_) => normal_components += 1,
            // Parent traversal makes lexical safety ambiguous (for example,
            // `/tmp/..` resolves to `/`). Reject it rather than guessing.
            Component::ParentDir => {
                return Err(anyhow::anyhow!(
                    "Refusing to remove path containing '..': {}",
                    path
                ));
            }
            Component::RootDir | Component::CurDir => {}
            Component::Prefix(_) => {
                return Err(anyhow::anyhow!(
                    "Refusing to remove unsupported path: {}",
                    path
                ));
            }
        }
    }

    if normal_components == 0 {
        return Err(anyhow::anyhow!(
            "Refusing to remove dangerous path: {}",
            path
        ));
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileState {
    File,
    Directory,
    Link,
    Absent,
    Touch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathKind {
    Absent,
    File,
    Directory,
    Link,
    Other,
}

#[derive(Debug, Clone)]
pub(crate) struct PathStatus {
    pub kind: PathKind,
    mode: Option<String>,
    owner: Option<String>,
    group: Option<String>,
    uid: Option<String>,
    gid: Option<String>,
    pub link_target: Option<String>,
}

impl PathStatus {
    fn absent() -> Self {
        Self {
            kind: PathKind::Absent,
            mode: None,
            owner: None,
            group: None,
            uid: None,
            gid: None,
            link_target: None,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct MetadataChanges {
    mode: Option<String>,
    owner: Option<String>,
    group: Option<String>,
}

impl MetadataChanges {
    pub(crate) fn is_changed(&self) -> bool {
        self.mode.is_some() || self.owner.is_some() || self.group.is_some()
    }
}

impl FileState {
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "file" => Ok(FileState::File),
            "directory" | "dir" => Ok(FileState::Directory),
            "link" => Ok(FileState::Link),
            "absent" => Ok(FileState::Absent),
            "touch" => Ok(FileState::Touch),
            _ => Err(anyhow::anyhow!("Invalid file state: {}", s)),
        }
    }
}

/// Execute the file module logic: create/remove/touch/set permissions/ownership for files or directories.
pub fn execute(
    connection: &dyn SshConnection,
    file_args: &Value,
    use_become: bool,
    become_user: &str,
    check_mode: bool,
) -> Result<ModuleResult> {
    validate_params(
        file_args,
        &["path", "dest", "state", "src", "mode", "owner", "group"],
    )?;
    let map = file_args
        .as_mapping()
        .ok_or_else(|| anyhow::anyhow!("File module requires a mapping of arguments"))?;
    let path_key = Value::String("path".to_string());
    let dest_key = Value::String("dest".to_string());
    let path = match (map.get(&path_key), map.get(&dest_key)) {
        (Some(Value::String(path)), None) | (None, Some(Value::String(path)))
            if !path.is_empty() =>
        {
            path.clone()
        }
        (Some(_), Some(_)) => {
            return Err(anyhow::anyhow!(
                "File parameters 'path' and 'dest' are mutually exclusive"
            ));
        }
        (None, None) => return Err(anyhow::anyhow!("File module requires 'path' or 'dest'")),
        _ => return Err(anyhow::anyhow!("File path must be a non-empty string")),
    };
    let state = match map.get(Value::String("state".to_string())) {
        Some(Value::String(state)) => FileState::from_str(state)?,
        Some(_) => return Err(anyhow::anyhow!("File state must be a string")),
        None => FileState::File,
    };
    let mode = get_optional_param::<String>(file_args, "mode")?;
    let owner = get_optional_param::<String>(file_args, "owner")?;
    let group = get_optional_param::<String>(file_args, "group")?;

    let current = inspect_path(connection, &path, use_become, become_user)?;
    let mut object_changed = false;
    let mut replacement = false;

    match state {
        FileState::File => match current.kind {
            PathKind::File => {}
            PathKind::Absent => {
                return Err(anyhow::anyhow!(
                    "File {} does not exist (state=file does not create files)",
                    path
                ));
            }
            _ => {
                return Err(anyhow::anyhow!(
                    "Path {} exists but is not a regular file",
                    path
                ));
            }
        },
        FileState::Directory => match current.kind {
            PathKind::Directory => {}
            PathKind::Absent => {
                object_changed = true;
                replacement = true;
                if !check_mode {
                    create_directory(connection, &path, use_become, become_user)?;
                }
            }
            _ => {
                return Err(anyhow::anyhow!(
                    "Path {} exists but is not a directory",
                    path
                ));
            }
        },
        FileState::Absent => {
            if current.kind != PathKind::Absent {
                validate_removal_path(&path)?;
                object_changed = true;
                if !check_mode {
                    remove_file(connection, &path, use_become, become_user)?;
                }
            }
        }
        FileState::Touch => {
            object_changed = true;
            replacement = current.kind == PathKind::Absent;
            if !check_mode {
                touch_file(connection, &path, use_become, become_user)?;
            }
        }
        FileState::Link => {
            let src = get_param::<String>(file_args, "src")?;
            match current.kind {
                PathKind::Link if current.link_target.as_deref() == Some(src.as_str()) => {}
                PathKind::Link | PathKind::Absent => {
                    object_changed = true;
                    replacement = true;
                    if !check_mode {
                        create_symlink(connection, &src, &path, use_become, become_user)?;
                    }
                }
                _ => {
                    return Err(anyhow::anyhow!(
                        "Path {} already exists and is not a symbolic link",
                        path
                    ));
                }
            }
        }
    }

    let metadata_changes = if matches!(state, FileState::Absent) {
        MetadataChanges::default()
    } else {
        // A freshly created/replaced path has unknown/default metadata, so all
        // explicitly requested metadata must be applied.
        let metadata_basis = if replacement {
            let mut status = PathStatus::absent();
            if matches!(state, FileState::Link) {
                status.kind = PathKind::Link;
            }
            status
        } else {
            current.clone()
        };
        plan_metadata_changes(
            &metadata_basis,
            mode.as_deref(),
            owner.as_deref(),
            group.as_deref(),
        )?
    };
    let changed = object_changed || metadata_changes.is_changed();

    if metadata_changes.is_changed() && !check_mode {
        apply_metadata_changes(
            connection,
            &path,
            &metadata_changes,
            matches!(state, FileState::Link),
            use_become,
            become_user,
        )?;
    }

    info!("File operation completed successfully");
    Ok(ModuleResult {
        stdout: String::new(),
        stderr: String::new(),
        rc: None,
        changed,
        failed: false,
        msg: if changed {
            format!(
                "File {} {}",
                path,
                if check_mode {
                    "would be updated"
                } else {
                    "updated"
                }
            )
        } else {
            format!("File {} is already in the requested state", path)
        },
        values: Default::default(),
    })
}

fn execute_inspection_command(
    connection: &dyn SshConnection,
    command: &str,
    use_become: bool,
    become_user: &str,
) -> Result<String> {
    let (exit_code, stdout, stderr) = if use_become {
        connection.execute_sudo_command(command, become_user)?
    } else {
        connection.execute_command(command)?
    };
    if exit_code != 0 {
        return Err(anyhow::anyhow!(
            "Failed to inspect path (exit code {}): {}",
            exit_code,
            stderr.trim()
        ));
    }
    Ok(stdout)
}

pub(crate) fn inspect_path(
    connection: &dyn SshConnection,
    path: &str,
    use_become: bool,
    become_user: &str,
) -> Result<PathStatus> {
    let quoted_path = quote_posix_shell_arg(path)?;
    let kind_command = format!(
        "if [ -L {0} ]; then printf 'link\\n'; elif [ -f {0} ]; then printf 'file\\n'; elif [ -d {0} ]; then printf 'directory\\n'; elif [ -e {0} ]; then printf 'other\\n'; else printf 'absent\\n'; fi",
        quoted_path
    );
    let kind = match execute_inspection_command(connection, &kind_command, use_become, become_user)?
        .trim()
    {
        "absent" => PathKind::Absent,
        "file" => PathKind::File,
        "directory" => PathKind::Directory,
        "link" => PathKind::Link,
        "other" => PathKind::Other,
        output => return Err(anyhow::anyhow!("Unexpected path type response: {}", output)),
    };

    if kind == PathKind::Absent {
        return Ok(PathStatus::absent());
    }

    let metadata_command = format!("stat -c '%a|%U|%G|%u|%g' -- {}", quoted_path);
    let metadata =
        execute_inspection_command(connection, &metadata_command, use_become, become_user)?;
    let fields = metadata
        .trim_end_matches(['\r', '\n'])
        .split('|')
        .collect::<Vec<_>>();
    if fields.len() != 5 {
        return Err(anyhow::anyhow!("Unexpected metadata response for {}", path));
    }

    let link_target = if kind == PathKind::Link {
        let command = format!("readlink -- {}", quoted_path);
        Some(
            execute_inspection_command(connection, &command, use_become, become_user)?
                .trim_end_matches(['\r', '\n'])
                .to_string(),
        )
    } else {
        None
    };

    Ok(PathStatus {
        kind,
        mode: Some(fields[0].to_string()),
        owner: Some(fields[1].to_string()),
        group: Some(fields[2].to_string()),
        uid: Some(fields[3].to_string()),
        gid: Some(fields[4].to_string()),
        link_target,
    })
}

fn normalize_mode(mode: &str) -> Result<String> {
    if mode.is_empty() || !mode.chars().all(|character| matches!(character, '0'..='7')) {
        return Err(anyhow::anyhow!(
            "File mode must be a non-empty octal string"
        ));
    }
    let parsed = u32::from_str_radix(mode, 8).map_err(|_| anyhow::anyhow!("Invalid mode"))?;
    if parsed > 0o7777 {
        return Err(anyhow::anyhow!("File mode is outside the supported range"));
    }
    Ok(format!("{:o}", parsed))
}

pub(crate) fn plan_metadata_changes(
    current: &PathStatus,
    mode: Option<&str>,
    owner: Option<&str>,
    group: Option<&str>,
) -> Result<MetadataChanges> {
    // POSIX symlink permission bits cannot be changed portably. Ownership is
    // still managed with `chown -h` below.
    let mode = if current.kind == PathKind::Link {
        None
    } else if let Some(desired) = mode {
        let normalized = normalize_mode(desired)?;
        (current.mode.as_deref() != Some(normalized.as_str())).then_some(desired.to_string())
    } else {
        None
    };

    let owner = owner.and_then(|desired| {
        let matches =
            current.owner.as_deref() == Some(desired) || current.uid.as_deref() == Some(desired);
        (!matches).then_some(desired.to_string())
    });
    let group = group.and_then(|desired| {
        let matches =
            current.group.as_deref() == Some(desired) || current.gid.as_deref() == Some(desired);
        (!matches).then_some(desired.to_string())
    });

    Ok(MetadataChanges { mode, owner, group })
}

pub(crate) fn apply_metadata_changes(
    connection: &dyn SshConnection,
    path: &str,
    changes: &MetadataChanges,
    is_link: bool,
    use_become: bool,
    become_user: &str,
) -> Result<()> {
    let quoted_path = quote_posix_shell_arg(path)?;
    if let Some(mode) = changes.mode.as_deref() {
        let command = format!("chmod -- {} {}", quote_posix_shell_arg(mode)?, quoted_path);
        execute_mutating_command(
            connection,
            &command,
            use_become,
            become_user,
            "set file mode",
        )?;
    }

    if changes.owner.is_some() || changes.group.is_some() {
        let ownership = match (changes.owner.as_deref(), changes.group.as_deref()) {
            (Some(o), Some(g)) => format!("{}:{}", o, g),
            (Some(o), None) => o.to_string(),
            (None, Some(g)) => format!(":{}", g),
            (None, None) => return Ok(()),
        };
        let link_flag = if is_link { "-h " } else { "" };
        let command = format!(
            "chown {}-- {} {}",
            link_flag,
            quote_posix_shell_arg(&ownership)?,
            quoted_path
        );
        execute_mutating_command(
            connection,
            &command,
            use_become,
            become_user,
            "set file ownership",
        )?;
    }
    Ok(())
}

fn execute_mutating_command(
    connection: &dyn SshConnection,
    command: &str,
    use_become: bool,
    become_user: &str,
    operation: &str,
) -> Result<()> {
    let (exit_code, _, stderr) = if use_become {
        connection.execute_sudo_command(command, become_user)?
    } else {
        connection.execute_command(command)?
    };
    if exit_code != 0 {
        Err(anyhow::anyhow!(
            "Failed to {}: {}",
            operation,
            stderr.trim()
        ))
    } else {
        Ok(())
    }
}

/// Create a directory if it does not exist.
fn create_directory(
    connection: &dyn SshConnection,
    path: &str,
    use_become: bool,
    become_user: &str,
) -> Result<()> {
    info!("Creating a directory");
    let quoted_path = quote_posix_shell_arg(path)?;
    let cmd = format!("mkdir -p -- {}", quoted_path);
    let (exit_code, _, stderr) = if use_become {
        connection.execute_sudo_command(&cmd, become_user)?
    } else {
        connection.execute_command(&cmd)?
    };
    if exit_code != 0 {
        return Err(anyhow::anyhow!("Failed to create directory: {}", stderr));
    }
    Ok(())
}

/// Remove a file or directory.
fn remove_file(
    connection: &dyn SshConnection,
    path: &str,
    use_become: bool,
    become_user: &str,
) -> Result<()> {
    info!("Removing a file or directory");
    validate_removal_path(path)?;
    let quoted_path = quote_posix_shell_arg(path)?;
    let cmd = format!("rm -rf -- {}", quoted_path);
    let (exit_code, _, stderr) = if use_become {
        connection.execute_sudo_command(&cmd, become_user)?
    } else {
        connection.execute_command(&cmd)?
    };
    if exit_code != 0 {
        return Err(anyhow::anyhow!(
            "Failed to remove file/directory: {}",
            stderr
        ));
    }
    Ok(())
}

/// Touch a file (update timestamp or create if not exists).
fn touch_file(
    connection: &dyn SshConnection,
    path: &str,
    use_become: bool,
    become_user: &str,
) -> Result<()> {
    info!("Updating a file timestamp");
    let quoted_path = quote_posix_shell_arg(path)?;
    let cmd = format!("touch -- {}", quoted_path);
    let (exit_code, _, stderr) = if use_become {
        connection.execute_sudo_command(&cmd, become_user)?
    } else {
        connection.execute_command(&cmd)?
    };
    if exit_code != 0 {
        return Err(anyhow::anyhow!("Failed to touch file: {}", stderr));
    }
    Ok(())
}

/// Create a symbolic link.
fn create_symlink(
    connection: &dyn SshConnection,
    src: &str,
    dest: &str,
    use_become: bool,
    become_user: &str,
) -> Result<()> {
    info!("Creating a symbolic link");
    let quoted_src = quote_posix_shell_arg(src)?;
    let quoted_dest = quote_posix_shell_arg(dest)?;
    let cmd = format!("ln -sfn -- {} {}", quoted_src, quoted_dest);
    let (exit_code, _, stderr) = if use_become {
        connection.execute_sudo_command(&cmd, become_user)?
    } else {
        connection.execute_command(&cmd)?
    };
    if exit_code != 0 {
        return Err(anyhow::anyhow!("Failed to create symlink: {}", stderr));
    }
    Ok(())
}

/// Execute the file module in ad-hoc mode for a single host.
pub fn execute_adhoc(
    host: &Host,
    file_args: &Value,
    use_become: bool,
    become_user: &str,
    check_mode: bool,
) -> Result<ModuleResult> {
    info!("Connecting to host: {}", host.name);
    let connection = Connection::connect(host)?;
    execute(
        connection.as_connection(),
        file_args,
        use_become,
        become_user,
        check_mode,
    )
}

#[cfg(test)]
mod tests {
    use super::{execute, quote_posix_shell_arg, validate_removal_path};
    use crate::inventory::Host;
    use crate::ssh::connection::LocalConnection;
    use serde_yaml::{Mapping, Value};

    fn file_args(path: &str, state: &str) -> Value {
        let mut map = Mapping::new();
        map.insert(Value::String("path".into()), Value::String(path.into()));
        map.insert(Value::String("state".into()), Value::String(state.into()));
        Value::Mapping(map)
    }

    fn local_connection() -> LocalConnection {
        LocalConnection::new(&Host::new("localhost")).unwrap()
    }

    #[test]
    fn test_file_param_extract_ok() {
        let mut map = Mapping::new();
        map.insert(
            Value::String("path".to_string()),
            Value::String("/tmp/f".to_string()),
        );
        let args = Value::Mapping(map);
        assert_eq!(
            crate::modules::param::get_param::<String>(&args, "path").unwrap(),
            "/tmp/f"
        );
    }

    #[test]
    fn test_file_param_missing() {
        let map = Mapping::new();
        let args = Value::Mapping(map);
        assert!(crate::modules::param::get_param::<String>(&args, "path").is_err());
    }

    #[test]
    fn test_file_state_parsing() {
        use super::FileState;
        assert_eq!(FileState::from_str("file").unwrap(), FileState::File);
        assert_eq!(
            FileState::from_str("directory").unwrap(),
            FileState::Directory
        );
        assert_eq!(FileState::from_str("dir").unwrap(), FileState::Directory);
        assert_eq!(FileState::from_str("link").unwrap(), FileState::Link);
        assert_eq!(FileState::from_str("absent").unwrap(), FileState::Absent);
        assert_eq!(FileState::from_str("touch").unwrap(), FileState::Touch);
        assert!(FileState::from_str("invalid").is_err());
    }

    #[test]
    fn test_posix_shell_argument_quoting() {
        assert_eq!(quote_posix_shell_arg("simple").unwrap(), "'simple'");
        assert_eq!(
            quote_posix_shell_arg("a b;$(id)'tail").unwrap(),
            "'a b;$(id)'\"'\"'tail'"
        );
        assert!(quote_posix_shell_arg("bad\0value").is_err());
    }

    #[test]
    fn test_rejects_dangerous_removal_paths() {
        for path in ["", "   ", "/", "//", ".", "./", "..", "../", "/tmp/.."] {
            assert!(validate_removal_path(path).is_err(), "accepted {path:?}");
        }

        for path in [
            "relative/file",
            "/tmp/file",
            "./nested/file",
            "name with spaces",
        ] {
            assert!(validate_removal_path(path).is_ok(), "rejected {path:?}");
        }
    }

    #[test]
    fn directory_is_idempotent_and_check_mode_does_not_create() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("managed directory");
        let mut args = match file_args(path.to_str().unwrap(), "directory") {
            Value::Mapping(map) => map,
            _ => unreachable!(),
        };
        args.insert(Value::String("mode".into()), Value::String("0700".into()));
        let args = Value::Mapping(args);
        let connection = local_connection();

        let first = execute(&connection, &args, false, "", false).unwrap();
        assert!(first.changed);
        assert!(path.is_dir());

        let second = execute(&connection, &args, false, "", false).unwrap();
        assert!(!second.changed);

        let checked_path = directory.path().join("check only");
        let checked_args = file_args(checked_path.to_str().unwrap(), "directory");
        let checked = execute(&connection, &checked_args, false, "", true).unwrap();
        assert!(checked.changed);
        assert!(!checked_path.exists());
    }

    #[test]
    fn state_file_does_not_create_a_missing_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("missing");
        let args = file_args(path.to_str().unwrap(), "file");

        let error = execute(&local_connection(), &args, false, "", false)
            .err()
            .unwrap();
        assert!(error.to_string().contains("does not exist"));
        assert!(!path.exists());
    }

    #[test]
    fn link_target_is_idempotent_and_check_mode_only_predicts() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("managed link");
        let mut args = match file_args(path.to_str().unwrap(), "link") {
            Value::Mapping(map) => map,
            _ => unreachable!(),
        };
        args.insert(
            Value::String("src".into()),
            Value::String("first target".into()),
        );
        let connection = local_connection();

        assert!(
            execute(&connection, &Value::Mapping(args.clone()), false, "", false)
                .unwrap()
                .changed
        );
        assert!(
            !execute(&connection, &Value::Mapping(args.clone()), false, "", false)
                .unwrap()
                .changed
        );

        args.insert(
            Value::String("src".into()),
            Value::String("second target".into()),
        );
        assert!(
            execute(&connection, &Value::Mapping(args), false, "", true)
                .unwrap()
                .changed
        );
        assert_eq!(
            std::fs::read_link(path).unwrap(),
            std::path::Path::new("first target")
        );
    }
}
