use anyhow::{Context, Result};
use log::info;
use serde_yaml::Value;
use std::path::Path;

use crate::inventory::Host;
use crate::modules::file::{apply_metadata_changes, inspect_path, plan_metadata_changes, PathKind};
use crate::modules::param::{get_optional_param, get_param, validate_params};
use crate::modules::ModuleExecutor;
use crate::modules::ModuleResult;
use crate::ssh::connection::SshConnection;

pub struct CopyModule;

impl ModuleExecutor for CopyModule {
    fn execute(
        connection: &dyn SshConnection,
        copy_args: &Value,
        use_become: bool,
        become_user: &str,
        check_mode: bool,
    ) -> Result<ModuleResult> {
        validate_params(
            copy_args,
            &["src", "content", "dest", "mode", "owner", "group"],
        )?;
        let dest = get_param::<String>(copy_args, "dest")?;
        if dest.is_empty() {
            return Err(anyhow::anyhow!("Copy destination cannot be empty"));
        }

        // Extract optional parameters
        let mode = get_optional_param::<String>(copy_args, "mode")?;
        let owner = get_optional_param::<String>(copy_args, "owner")?;
        let group = get_optional_param::<String>(copy_args, "group")?;

        // Determine content source
        let content: Vec<u8> = if let Value::Mapping(args_map) = copy_args {
            let src_value = args_map.get(Value::String("src".to_string()));
            let content_value = args_map.get(Value::String("content".to_string()));
            if src_value.is_some() && content_value.is_some() {
                return Err(anyhow::anyhow!(
                    "Copy parameters 'src' and 'content' are mutually exclusive"
                ));
            }
            if let Some(content_value) = content_value {
                // Content provided directly
                match content_value {
                    Value::String(s) => s.as_bytes().to_vec(),
                    _ => return Err(anyhow::anyhow!("Copy content must be a string")),
                }
            } else if src_value.is_some() {
                // Content from file
                let src = get_param::<String>(copy_args, "src")?;
                info!("Reading content from a source file");

                // Check if source file exists locally
                let src_path = Path::new(&src);
                if !src_path.exists() {
                    return Err(anyhow::anyhow!("Source file does not exist: {}", src));
                }

                // Preserve arbitrary file bytes; copy is not a text-only module.
                std::fs::read(&src)
                    .with_context(|| format!("Failed to read source file: {}", src))?
            } else {
                return Err(anyhow::anyhow!("Copy requires either 'src' or 'content'"));
            }
        } else {
            return Err(anyhow::anyhow!(
                "Copy module requires a mapping of arguments"
            ));
        };

        let current = inspect_path(connection, &dest, use_become, become_user)?;
        if !matches!(current.kind, PathKind::Absent | PathKind::File) {
            return Err(anyhow::anyhow!(
                "Copy destination {} exists but is not a regular file",
                dest
            ));
        }
        let existing_content = if use_become {
            connection.read_file_bytes_with_sudo(&dest, become_user)?
        } else {
            connection.read_file_bytes(&dest)?
        };
        let content_changed = existing_content.as_deref() != Some(content.as_slice());
        let metadata_changes = plan_metadata_changes(
            &current,
            mode.as_deref(),
            owner.as_deref(),
            group.as_deref(),
        )?;
        let changed = content_changed || metadata_changes.is_changed();

        if changed && !check_mode {
            if content_changed {
                if use_become {
                    // The privileged install path may replace the inode, so
                    // pass all requested metadata rather than only differences
                    // measured on the previous destination.
                    connection.write_file_bytes_with_sudo(
                        &content,
                        &dest,
                        become_user,
                        mode.clone(),
                        owner.clone(),
                        group.clone(),
                    )?;
                } else {
                    connection.write_file_bytes(&dest, &content)?;
                }
            }

            if metadata_changes.is_changed() && (!use_become || !content_changed) {
                apply_metadata_changes(
                    connection,
                    &dest,
                    &metadata_changes,
                    false,
                    use_become,
                    become_user,
                )?;
            }
        }

        let source_info = if let Value::Mapping(args_map) = copy_args {
            if args_map.get(Value::String("content".to_string())).is_some() {
                "inline content".to_string()
            } else if let Some(Value::String(src)) = args_map.get(Value::String("src".to_string()))
            {
                src.clone()
            } else {
                "unknown source".to_string()
            }
        } else {
            "unknown source".to_string()
        };

        Ok(ModuleResult {
            stdout: String::new(),
            stderr: String::new(),
            rc: None,
            changed,
            failed: false,
            msg: if changed {
                format!("Content copied from {} to {}", source_info, dest)
            } else {
                format!("Destination {} is already up to date", dest)
            },
        })
    }
}

pub fn execute(
    connection: &dyn SshConnection,
    copy_args: &Value,
    use_become: bool,
    become_user: &str,
    check_mode: bool,
) -> Result<ModuleResult> {
    CopyModule::execute(connection, copy_args, use_become, become_user, check_mode)
}

pub fn execute_adhoc(
    host: &Host,
    copy_args: &Value,
    use_become: bool,
    become_user: &str,
    check_mode: bool,
) -> Result<ModuleResult> {
    CopyModule::execute_adhoc(host, copy_args, use_become, become_user, check_mode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::connection::LocalConnection;
    use serde_yaml::{Mapping, Value};

    fn copy_args(src: &str, dest: &str) -> Value {
        let mut map = Mapping::new();
        map.insert(Value::String("src".into()), Value::String(src.into()));
        map.insert(Value::String("dest".into()), Value::String(dest.into()));
        Value::Mapping(map)
    }

    fn local_connection() -> LocalConnection {
        LocalConnection::new(&Host::new("localhost")).unwrap()
    }

    #[test]
    fn test_extract_params() {
        let mut map = Mapping::new();
        map.insert(
            Value::String("src".to_string()),
            Value::String("/tmp/source".to_string()),
        );
        map.insert(
            Value::String("dest".to_string()),
            Value::String("/tmp/dest".to_string()),
        );
        let args = Value::Mapping(map);

        assert_eq!(get_param::<String>(&args, "src").unwrap(), "/tmp/source");
        assert_eq!(get_param::<String>(&args, "dest").unwrap(), "/tmp/dest");
    }

    #[test]
    fn test_content_param() {
        let mut map = Mapping::new();
        map.insert(
            Value::String("content".to_string()),
            Value::String("Hello, world!".to_string()),
        );
        map.insert(
            Value::String("dest".to_string()),
            Value::String("/tmp/test-content".to_string()),
        );
        let args = Value::Mapping(map);

        if let Value::Mapping(args_map) = &args {
            if let Some(content_value) = args_map.get(Value::String("content".to_string())) {
                match content_value {
                    Value::String(s) => assert_eq!(s, "Hello, world!"),
                    _ => panic!("Content value is not a string"),
                }
            } else {
                panic!("Content key not found");
            }
        } else {
            panic!("Args is not a mapping");
        }
    }

    #[test]
    fn binary_copy_is_idempotent_and_check_mode_does_not_write() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        let destination = directory.path().join("destination");
        let original = [0_u8, 0xff, b'\n', 0x80];
        std::fs::write(&source, original).unwrap();
        let args = copy_args(source.to_str().unwrap(), destination.to_str().unwrap());
        let connection = local_connection();

        let first = execute(&connection, &args, false, "", false).unwrap();
        assert!(first.changed);
        assert_eq!(std::fs::read(&destination).unwrap(), original);

        let second = execute(&connection, &args, false, "", false).unwrap();
        assert!(!second.changed);

        let replacement = [b'n', b'e', b'w', 0_u8, 0xfe];
        std::fs::write(&source, replacement).unwrap();
        let checked = execute(&connection, &args, false, "", true).unwrap();
        assert!(checked.changed);
        assert_eq!(std::fs::read(&destination).unwrap(), original);
    }
}
