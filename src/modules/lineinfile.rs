use anyhow::{bail, Context, Result};
use log::info;
use regex::Regex;
use serde_yaml::Value;

use crate::inventory::Host;
use crate::modules::file::{
    apply_metadata_changes, inspect_path, plan_metadata_changes, quote_posix_shell_arg, PathKind,
};
use crate::modules::param::{get_optional_param, get_param, validate_params};
use crate::modules::ModuleResult;
use crate::ssh::connection::{Connection, SshConnection};

fn run(
    connection: &dyn SshConnection,
    command: &str,
    use_become: bool,
    become_user: &str,
) -> Result<(i32, String, String)> {
    if use_become {
        connection.execute_sudo_command(command, become_user)
    } else {
        connection.execute_command(command)
    }
}

fn read_content(
    connection: &dyn SshConnection,
    path: &str,
    use_become: bool,
    become_user: &str,
) -> Result<String> {
    let bytes = if use_become {
        let command = format!("cat -- {}", quote_posix_shell_arg(path)?);
        let (code, stdout, stderr) = connection.execute_sudo_command(&command, become_user)?;
        if code != 0 {
            bail!("Failed to read file {}: {}", path, stderr.trim());
        }
        stdout.into_bytes()
    } else {
        connection
            .read_file_bytes(path)?
            .with_context(|| format!("File {} disappeared while it was being inspected", path))?
    };
    String::from_utf8(bytes).with_context(|| format!("File {} is not valid UTF-8", path))
}

fn write_content(
    connection: &dyn SshConnection,
    path: &str,
    content: &str,
    use_become: bool,
    become_user: &str,
) -> Result<()> {
    if use_become {
        // Feed file data over stdin: content is never interpolated into a
        // shell command, and the selected become user is preserved.
        let command = format!("tee -- {} >/dev/null", quote_posix_shell_arg(path)?);
        let (code, _, stderr) = connection.execute_sudo_command_with_input(
            &command,
            become_user,
            content.as_bytes(),
        )?;
        if code != 0 {
            bail!("Failed to write file {}: {}", path, stderr.trim());
        }
    } else {
        connection.write_file_content(path, content)?;
    }
    Ok(())
}

fn validate_new_metadata(
    mode: Option<&str>,
    owner: Option<&str>,
    group: Option<&str>,
) -> Result<()> {
    if let Some(mode) = mode {
        if mode.is_empty() || !mode.chars().all(|character| matches!(character, '0'..='7')) {
            bail!("File mode must be a non-empty octal string");
        }
        let parsed = u32::from_str_radix(mode, 8).context("Invalid file mode")?;
        if parsed > 0o7777 {
            bail!("File mode is outside the supported range");
        }
    }
    for (kind, value) in [("owner", owner), ("group", group)] {
        if value.is_some_and(str::is_empty) {
            bail!("File {kind} cannot be empty");
        }
    }
    Ok(())
}

fn validate_line(line: Option<&str>) -> Result<()> {
    if line.is_some_and(|value| value.contains(['\n', '\r'])) {
        bail!("'line' must be a single line and cannot contain newline characters");
    }
    Ok(())
}

pub fn execute(
    connection: &dyn SshConnection,
    args: &Value,
    use_become: bool,
    become_user: &str,
    check_mode: bool,
) -> Result<ModuleResult> {
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
    let path = get_param::<String>(args, "path")?;
    let line = get_optional_param::<String>(args, "line")?;
    let regexp = get_optional_param::<String>(args, "regexp")?;
    let state =
        get_optional_param::<String>(args, "state")?.unwrap_or_else(|| "present".to_string());
    let backup = get_optional_param::<bool>(args, "backup")?.unwrap_or(false);
    let create = get_optional_param::<bool>(args, "create")?.unwrap_or(false);
    let insertafter = get_optional_param::<String>(args, "insertafter")?;
    let insertbefore = get_optional_param::<String>(args, "insertbefore")?;
    let owner = get_optional_param::<String>(args, "owner")?;
    let group = get_optional_param::<String>(args, "group")?;
    let mode = get_optional_param::<String>(args, "mode")?;

    validate_line(line.as_deref())?;
    if insertafter.is_some() && insertbefore.is_some() {
        bail!("'insertafter' and 'insertbefore' are mutually exclusive");
    }
    if !matches!(state.as_str(), "present" | "absent") {
        bail!("Invalid state: {}. Must be 'present' or 'absent'", state);
    }

    info!("Managing a line in a file");
    let current = inspect_path(connection, &path, use_become, become_user)?;
    if !matches!(current.kind, PathKind::Absent | PathKind::File) {
        bail!("Path {} exists but is not a regular file", path);
    }
    let file_exists = current.kind == PathKind::File;
    if !file_exists && state == "present" && !create {
        bail!("File {} does not exist and create=false", path);
    }

    let mut content = if file_exists {
        read_content(connection, &path, use_become, become_user)?
    } else {
        String::new()
    };
    let content_changed = process_line_modifications(
        &mut content,
        line,
        regexp,
        &state,
        insertafter,
        insertbefore,
    )?;
    let creates_file = !file_exists && state == "present";

    validate_new_metadata(mode.as_deref(), owner.as_deref(), group.as_deref())?;
    let existing_metadata_changes = if file_exists {
        Some(plan_metadata_changes(
            &current,
            mode.as_deref(),
            owner.as_deref(),
            group.as_deref(),
        )?)
    } else {
        None
    };
    let metadata_changed = if state == "absent" && !file_exists {
        false
    } else {
        existing_metadata_changes
            .as_ref()
            .is_some_and(|changes| changes.is_changed())
            || (creates_file && (mode.is_some() || owner.is_some() || group.is_some()))
    };
    let changed = content_changed || creates_file || metadata_changed;

    if !check_mode {
        if content_changed || creates_file {
            // Backups correspond to content replacements only. Metadata-only
            // changes and check mode never create a backup.
            if backup && file_exists && content_changed {
                let backup_path = format!("{}.backup", path);
                let command = format!(
                    "cp -- {} {}",
                    quote_posix_shell_arg(&path)?,
                    quote_posix_shell_arg(&backup_path)?
                );
                let (code, _, stderr) = run(connection, &command, use_become, become_user)?;
                if code != 0 {
                    bail!("Failed to create backup {}: {}", backup_path, stderr.trim());
                }
            }
            write_content(connection, &path, &content, use_become, become_user)?;
        }

        if file_exists {
            if let Some(changes) = existing_metadata_changes.as_ref() {
                if changes.is_changed() {
                    apply_metadata_changes(
                        connection,
                        &path,
                        changes,
                        false,
                        use_become,
                        become_user,
                    )?;
                }
            }
        } else if creates_file && (mode.is_some() || owner.is_some() || group.is_some()) {
            // Inspect the actual defaults after creation, then apply only the
            // requested differences.
            let created = inspect_path(connection, &path, use_become, become_user)?;
            if created.kind != PathKind::File {
                bail!("Newly created path {} is not a regular file", path);
            }
            let changes = plan_metadata_changes(
                &created,
                mode.as_deref(),
                owner.as_deref(),
                group.as_deref(),
            )?;
            if changes.is_changed() {
                apply_metadata_changes(
                    connection,
                    &path,
                    &changes,
                    false,
                    use_become,
                    become_user,
                )?;
            }
        }
    }

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

fn process_line_modifications(
    content: &mut String,
    line: Option<String>,
    regexp: Option<String>,
    state: &str,
    insertafter: Option<String>,
    insertbefore: Option<String>,
) -> Result<bool> {
    validate_line(line.as_deref())?;

    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    let changed = match state {
        "present" => {
            let desired = line
                .as_ref()
                .context("'line' parameter is required when state=present")?;
            if let Some(pattern) = regexp {
                let regex =
                    Regex::new(&pattern).with_context(|| format!("Invalid regexp: {pattern}"))?;
                if let Some(existing) = lines.iter_mut().find(|entry| regex.is_match(entry)) {
                    if existing == desired {
                        false
                    } else {
                        *existing = desired.clone();
                        true
                    }
                } else {
                    insert_line(
                        &mut lines,
                        desired,
                        insertafter.as_deref(),
                        insertbefore.as_deref(),
                    )?;
                    true
                }
            } else if lines.iter().any(|entry| entry == desired) {
                false
            } else {
                insert_line(
                    &mut lines,
                    desired,
                    insertafter.as_deref(),
                    insertbefore.as_deref(),
                )?;
                true
            }
        }
        "absent" => {
            if let Some(pattern) = regexp {
                let regex =
                    Regex::new(&pattern).with_context(|| format!("Invalid regexp: {pattern}"))?;
                let original_len = lines.len();
                lines.retain(|entry| !regex.is_match(entry));
                lines.len() != original_len
            } else if let Some(desired) = line {
                let original_len = lines.len();
                lines.retain(|entry| entry != &desired);
                lines.len() != original_len
            } else {
                bail!("Either 'line' or 'regexp' is required when state=absent")
            }
        }
        _ => bail!("Invalid state: {state}"),
    };

    // Leave byte-for-byte content alone on a no-op. In particular, do not add
    // a trailing newline to an otherwise unchanged file.
    if changed {
        *content = lines.join("\n");
        if !content.is_empty() {
            content.push('\n');
        }
    }
    Ok(changed)
}

fn insert_line(
    lines: &mut Vec<String>,
    line: &str,
    insertafter: Option<&str>,
    insertbefore: Option<&str>,
) -> Result<()> {
    if let Some(pattern) = insertafter {
        if pattern == "EOF" {
            lines.push(line.to_string());
        } else {
            let regex = Regex::new(pattern)
                .with_context(|| format!("Invalid insertafter regexp: {pattern}"))?;
            let position = lines
                .iter()
                .position(|entry| regex.is_match(entry))
                .map_or(lines.len(), |index| index + 1);
            lines.insert(position, line.to_string());
        }
    } else if let Some(pattern) = insertbefore {
        if pattern == "BOF" {
            lines.insert(0, line.to_string());
        } else {
            let regex = Regex::new(pattern)
                .with_context(|| format!("Invalid insertbefore regexp: {pattern}"))?;
            let position = lines
                .iter()
                .position(|entry| regex.is_match(entry))
                .unwrap_or(lines.len());
            lines.insert(position, line.to_string());
        }
    } else {
        lines.push(line.to_string());
    }
    Ok(())
}

pub fn execute_adhoc(
    host: &Host,
    args: &Value,
    use_become: bool,
    become_user: &str,
    check_mode: bool,
) -> Result<ModuleResult> {
    info!("Opening connection for host: {}", host.name);
    let connection = Connection::connect(host)?;
    execute(
        connection.as_connection(),
        args,
        use_become,
        become_user,
        check_mode,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::{Mapping, Value};
    use std::fs;

    #[test]
    fn process_present_absent_and_regexp() {
        let mut content = "line1\nline2\nline3\n".to_string();
        assert!(process_line_modifications(
            &mut content,
            Some("replacement".to_string()),
            Some("^line2$".to_string()),
            "present",
            None,
            None,
        )
        .unwrap());
        assert_eq!(content, "line1\nreplacement\nline3\n");
        assert!(process_line_modifications(
            &mut content,
            None,
            Some("^line3$".to_string()),
            "absent",
            None,
            None,
        )
        .unwrap());
        assert_eq!(content, "line1\nreplacement\n");
    }

    #[test]
    fn no_op_preserves_missing_trailing_newline() {
        let mut content = "already present".to_string();
        assert!(!process_line_modifications(
            &mut content,
            Some("already present".to_string()),
            None,
            "present",
            None,
            None,
        )
        .unwrap());
        assert_eq!(content, "already present");
    }

    #[test]
    fn rejects_multiline_line_without_modifying_content() {
        for invalid_line in ["first\nsecond", "first\r\nsecond", "first\rsecond"] {
            let mut content = "existing\n".to_string();
            let error = process_line_modifications(
                &mut content,
                Some(invalid_line.to_string()),
                None,
                "present",
                None,
                None,
            )
            .unwrap_err()
            .to_string();

            assert!(error.contains("single line"), "{error}");
            assert_eq!(content, "existing\n");
        }
    }

    #[test]
    fn check_mode_does_not_write_or_backup() {
        let temporary = tempfile::NamedTempFile::new().unwrap();
        fs::write(temporary.path(), "before\n").unwrap();
        let host = Host::new("localhost");
        let connection = Connection::connect(&host).unwrap();
        let backup = format!("{}.backup", temporary.path().display());

        let mut args = Mapping::new();
        args.insert(
            Value::String("path".to_string()),
            Value::String(temporary.path().to_string_lossy().into_owned()),
        );
        args.insert(
            Value::String("line".to_string()),
            Value::String("after".to_string()),
        );
        args.insert(Value::String("backup".to_string()), Value::Bool(true));
        let result = execute(
            connection.as_connection(),
            &Value::Mapping(args),
            false,
            "",
            true,
        )
        .unwrap();

        assert!(result.changed);
        assert_eq!(fs::read_to_string(temporary.path()).unwrap(), "before\n");
        assert!(!std::path::Path::new(&backup).exists());
    }

    #[test]
    fn insertion_positions_and_absence_validation_match_ansible_edges() {
        for (after, before, expected) in [
            (Some("EOF"), None, "a\nb\nnew\n"),
            (Some("^a$"), None, "a\nnew\nb\n"),
            (Some("missing"), None, "a\nb\nnew\n"),
            (None, Some("BOF"), "new\na\nb\n"),
            (None, Some("^b$"), "a\nnew\nb\n"),
            (None, Some("missing"), "a\nb\nnew\n"),
        ] {
            let mut content = "a\nb\n".to_string();
            assert!(process_line_modifications(
                &mut content,
                Some("new".to_string()),
                None,
                "present",
                after.map(str::to_string),
                before.map(str::to_string),
            )
            .unwrap());
            assert_eq!(content, expected);
        }

        for (state, line, regexp) in [
            ("present", None, None),
            ("absent", None, None),
            ("invalid", Some("x".to_string()), None),
            ("present", Some("x".to_string()), Some("[".to_string())),
        ] {
            assert!(process_line_modifications(
                &mut String::new(),
                line,
                regexp,
                state,
                None,
                None,
            )
            .is_err());
        }
        assert!(insert_line(&mut vec![], "x", Some("["), None).is_err());
        assert!(insert_line(&mut vec![], "x", None, Some("[")).is_err());
    }

    #[test]
    fn lineinfile_creates_updates_backs_up_and_removes_lines_locally() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config");
        let connection = Connection::connect(&Host::new("localhost")).unwrap();
        let create = serde_yaml::from_str(&format!(
            "path: {}\nline: first\ncreate: true\nmode: '0600'",
            path.display()
        ))
        .unwrap();
        assert!(
            execute(connection.as_connection(), &create, false, "root", false)
                .unwrap()
                .changed
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "first\n");

        let replace = serde_yaml::from_str(&format!(
            "path: {}\nline: second\nregexp: '^first$'\nbackup: true",
            path.display()
        ))
        .unwrap();
        assert!(
            execute(connection.as_connection(), &replace, false, "root", false)
                .unwrap()
                .changed
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "second\n");
        assert_eq!(
            fs::read_to_string(format!("{}.backup", path.display())).unwrap(),
            "first\n"
        );

        let absent = serde_yaml::from_str(&format!(
            "path: {}\nline: second\nstate: absent",
            path.display()
        ))
        .unwrap();
        assert!(
            execute(connection.as_connection(), &absent, false, "root", false)
                .unwrap()
                .changed
        );
        assert_eq!(fs::read_to_string(path).unwrap(), "");
    }

    #[test]
    fn lineinfile_rejects_conflicting_and_invalid_file_requests() {
        let connection = Connection::connect(&Host::new("localhost")).unwrap();
        for input in [
            "path: /tmp/a\nline: x\ninsertafter: EOF\ninsertbefore: BOF",
            "path: /tmp/a\nline: x\nstate: invalid",
            "path: /definitely/missing\nline: x",
            "path: /tmp/a\nline: x\nmode: '0999'",
            "path: /tmp/a\nline: x\nowner: ''",
        ] {
            let args = serde_yaml::from_str(input).unwrap();
            assert!(execute(connection.as_connection(), &args, false, "root", false).is_err());
        }
    }
}
