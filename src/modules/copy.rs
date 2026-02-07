use anyhow::{Context, Result};
use log::info;
use serde_yaml::Value;
use std::path::Path;

use crate::inventory::Host;
use crate::modules::param::{get_optional_param, get_param};
use crate::modules::ModuleExecutor;
use crate::modules::ModuleResult;
use crate::ssh::connection::SshClient;

pub struct CopyModule;

impl ModuleExecutor for CopyModule {
    fn execute(
        ssh_client: &SshClient,
        copy_args: &Value,
        use_become: bool,
        _become_user: &str,
    ) -> Result<ModuleResult> {
        let dest = get_param::<String>(copy_args, "dest")?;

        // Extract optional parameters
        let mode = get_optional_param::<String>(copy_args, "mode");
        let owner = get_optional_param::<String>(copy_args, "owner");
        let group = get_optional_param::<String>(copy_args, "group");

        // Determine content source
        let content = if let Value::Mapping(args_map) = copy_args {
            if let Some(content_value) = args_map.get(&Value::String("content".to_string())) {
                // Content provided directly
                match content_value {
                    Value::String(s) => s.clone(),
                    _ => format!("{:?}", content_value),
                }
            } else {
                // Content from file
                let src = get_param::<String>(copy_args, "src")?;
                info!("Reading content from source file: {}", src);

                // Check if source file exists locally
                let src_path = Path::new(&src);
                if !src_path.exists() {
                    return Err(anyhow::anyhow!("Source file does not exist: {}", src));
                }

                // Read the source file content
                std::fs::read_to_string(&src)
                    .with_context(|| format!("Failed to read source file: {}", src))?
            }
        } else {
            return Err(anyhow::anyhow!(
                "Copy module requires a mapping of arguments"
            ));
        };

        info!(
            "Copying content to {}{}",
            dest,
            if use_become { " (with sudo)" } else { "" }
        );

        // Write file using appropriate method based on sudo requirement
        if use_become {
            // Use sudo-aware file writing method
            ssh_client.write_file_with_sudo(
                &content,
                &dest,
                mode.as_deref(),
                owner.as_deref(),
                group.as_deref(),
            )?;
        } else {
            // Write file normally
            ssh_client.write_file_content(&dest, &content)?;

            // Set permissions and ownership if specified (without sudo)
            if let Some(mode_str) = mode.as_deref() {
                let chmod_cmd = format!("chmod {} {}", mode_str, dest);
                let (exit_code, _, stderr) = ssh_client.execute_command(&chmod_cmd)?;
                if exit_code != 0 {
                    return Err(anyhow::anyhow!("Failed to set file mode: {}", stderr));
                }
            }

            if owner.is_some() || group.is_some() {
                let ownership = match (owner.as_deref(), group.as_deref()) {
                    (Some(o), Some(g)) => format!("{}:{}", o, g),
                    (Some(o), None) => o.to_string(),
                    (None, Some(g)) => format!(":{}", g),
                    (None, None) => String::new(),
                };

                if !ownership.is_empty() {
                    let chown_cmd = format!("chown {} {}", ownership, dest);
                    let (exit_code, _, stderr) = ssh_client.execute_command(&chown_cmd)?;
                    if exit_code != 0 {
                        return Err(anyhow::anyhow!("Failed to set file ownership: {}", stderr));
                    }
                }
            }
        }

        let source_info = if let Value::Mapping(args_map) = copy_args {
            if args_map
                .get(&Value::String("content".to_string()))
                .is_some()
            {
                "inline content".to_string()
            } else if let Some(Value::String(src)) = args_map.get(&Value::String("src".to_string()))
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
            changed: true,
            failed: false,
            msg: format!("Content copied from {} to {}", source_info, dest),
        })
    }
}

pub fn execute(
    ssh_client: &SshClient,
    copy_args: &Value,
    use_become: bool,
    become_user: &str,
) -> Result<ModuleResult> {
    CopyModule::execute(ssh_client, copy_args, use_become, become_user)
}

pub fn execute_adhoc(host: &Host, copy_args: &Value) -> Result<ModuleResult> {
    CopyModule::execute_adhoc(host, copy_args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::{Mapping, Value};

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
            if let Some(content_value) = args_map.get(&Value::String("content".to_string())) {
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
}
