use anyhow::{Context, Result};
use log::{debug, error, info};
use serde_json;
use serde_yaml::Value;
use std::error::Error;
use std::fs;
use std::path::Path;
use tera::{Context as TeraContext, Tera};

use crate::inventory::Host;
use crate::modules::file::{apply_metadata_changes, inspect_path, plan_metadata_changes, PathKind};
use crate::modules::param::{get_optional_param, get_param, validate_params};
use crate::modules::ModuleResult;
use crate::playbook::filters::register_ansible_filters;
use crate::ssh::connection::{Connection, SshConnection};

/// Execute the template module logic: render and upload a template, set permissions/ownership if needed.
pub fn execute(
    connection: &dyn SshConnection,
    template_args: &Value,
    use_become: bool,
    become_user: &str,
    check_mode: bool,
) -> Result<ModuleResult> {
    validate_params(
        template_args,
        &["src", "content", "dest", "mode", "owner", "group", "vars"],
    )?;
    let dest = get_param::<String>(template_args, "dest")?;
    if dest.is_empty() {
        return Err(anyhow::anyhow!("Template destination cannot be empty"));
    }

    // Extract optional parameters
    let mode = get_optional_param::<String>(template_args, "mode")?;
    let owner = get_optional_param::<String>(template_args, "owner")?;
    let group = get_optional_param::<String>(template_args, "group")?;

    // 解析模板内容 - 可以来自文件或直接内容
    let template_string: String;

    // 检查是否提供了内联内容
    if let Value::Mapping(args_map) = template_args {
        let src_value = args_map.get(Value::String("src".to_string()));
        let content_value = args_map.get(Value::String("content".to_string()));
        if src_value.is_some() && content_value.is_some() {
            return Err(anyhow::anyhow!(
                "Template parameters 'src' and 'content' are mutually exclusive"
            ));
        }
        if let Some(content_value) = content_value {
            template_string = match content_value {
                Value::String(s) => s.clone(),
                _ => return Err(anyhow::anyhow!("Template content must be a string")),
            };
        } else if let Some(Value::String(src)) = src_value {
            // 从文件读取模板内容
            let src_path = Path::new(src);
            if !src_path.exists() {
                return Err(anyhow::anyhow!("Template file does not exist: {}", src));
            }
            template_string = fs::read_to_string(src)
                .with_context(|| format!("Failed to read template file: {}", src))?;
        } else if src_value.is_some() {
            return Err(anyhow::anyhow!("Template src must be a string"));
        } else {
            return Err(anyhow::anyhow!(
                "Template requires either 'src' or 'content' parameter"
            ));
        }
    } else {
        return Err(anyhow::anyhow!("Template arguments must be a mapping"));
    }

    info!("Rendering a template");

    // 创建 Tera 上下文
    let mut tera_context = TeraContext::new();

    // Extract vars parameter if present and convert ALL variables to Tera context
    if let Value::Mapping(args_map) = template_args {
        if let Some(vars_value) = args_map.get(Value::String("vars".to_string())) {
            debug!(
                "Found 'vars' parameter with {} items",
                if let Value::Mapping(m) = vars_value {
                    m.len()
                } else {
                    0
                }
            );

            if let Value::Mapping(vars_map) = vars_value {
                // 首先创建一个临时的Tera上下文用于预渲染变量
                let temp_tera = Tera::default();
                let mut temp_context = TeraContext::new();

                // 先添加所有简单变量到临时上下文
                for (key, value) in vars_map {
                    let Value::String(key_str) = key else {
                        return Err(anyhow::anyhow!("Template variable names must be strings"));
                    };
                    let dynamic = matches!(
                        value,
                        Value::String(value) if value.contains("{{") || value.contains("{%")
                    );
                    if !dynamic {
                        let json_value = serde_json::to_value(value).with_context(|| {
                            format!("Failed to convert template variable '{}'", key_str)
                        })?;
                        temp_context.insert(key_str.clone(), &json_value);
                        tera_context.insert(key_str.clone(), &json_value);
                    }
                }

                // 第二遍：处理包含模板表达式的字符串变量
                for (key, value) in vars_map {
                    if let Value::String(key_str) = key {
                        if let Value::String(s) = value {
                            if s.contains("{{") || s.contains("{%") {
                                debug!(
                                    "Rendering template variable '{}' ({} bytes)",
                                    key_str,
                                    s.len()
                                );

                                // 尝试渲染包含模板表达式的字符串
                                let rendered_value = temp_tera
                                    .render_str(s, &temp_context, false)
                                    .with_context(|| {
                                        format!(
                                            "Failed to pre-render template variable '{}'",
                                            key_str
                                        )
                                    })?;
                                tera_context.insert(key_str.clone(), &rendered_value);
                                temp_context.insert(key_str.clone(), &rendered_value);
                            }
                        }
                    }
                }
            } else {
                return Err(anyhow::anyhow!(
                    "Template 'vars' parameter must be a mapping"
                ));
            }
        } else {
            debug!("No 'vars' parameter found in template arguments");
        }
    }

    debug!("Final Tera context has been populated with variables");

    // Render the template
    let mut tera = Tera::default();
    register_ansible_filters(&mut tera);

    let rendered_content = tera
        .render_str(&template_string, &tera_context, false)
        .map_err(|e| {
            error!("Template rendering failed");
            debug!("Template rendering context variables were available");

            // Try to provide more detailed error information
            let error_msg = format!("Template rendering failed: {}", e);
            if let Some(source) = e.source() {
                anyhow::anyhow!("{}\nCaused by: {}", error_msg, source)
            } else {
                anyhow::anyhow!(error_msg)
            }
        })?;

    let current = inspect_path(connection, &dest, use_become, become_user)?;
    if !matches!(current.kind, PathKind::Absent | PathKind::File) {
        return Err(anyhow::anyhow!(
            "Template destination {} exists but is not a regular file",
            dest
        ));
    }
    let rendered_bytes = rendered_content.as_bytes();
    let existing_content = if use_become {
        connection.read_file_bytes_with_sudo(&dest, become_user)?
    } else {
        connection.read_file_bytes(&dest)?
    };
    let content_changed = existing_content.as_deref() != Some(rendered_bytes);
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
                connection.write_file_bytes_with_sudo(
                    rendered_bytes,
                    &dest,
                    become_user,
                    mode.clone(),
                    owner.clone(),
                    group.clone(),
                )?;
            } else {
                connection.write_file_bytes(&dest, rendered_bytes)?;
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

    info!("Template rendering completed successfully");
    Ok(ModuleResult {
        stdout: String::new(),
        stderr: String::new(),
        rc: None,
        changed,
        failed: false,
        msg: if changed {
            format!("Template applied to {}", dest)
        } else {
            format!("Template destination {} is already up to date", dest)
        },
        values: Default::default(),
    })
}

/// Execute the template module in ad-hoc mode for a single host.
pub fn execute_adhoc(
    host: &Host,
    template_args: &Value,
    use_become: bool,
    become_user: &str,
    check_mode: bool,
) -> Result<ModuleResult> {
    info!("Opening connection for host: {}", host.name);
    let connection = Connection::connect(host)?;

    execute(
        connection.as_connection(),
        template_args,
        use_become,
        become_user,
        check_mode,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::connection::LocalConnection;
    use serde_yaml::{Mapping, Value};

    fn template_args(content: &str, dest: &str) -> Value {
        let mut map = Mapping::new();
        map.insert(
            Value::String("content".into()),
            Value::String(content.into()),
        );
        map.insert(Value::String("dest".into()), Value::String(dest.into()));
        Value::Mapping(map)
    }

    fn local_connection() -> LocalConnection {
        LocalConnection::new(&Host::new("localhost")).unwrap()
    }

    #[test]
    fn test_template_params() {
        let mut map = Mapping::new();
        map.insert(
            Value::String("dest".to_string()),
            Value::String("/tmp/test".to_string()),
        );
        map.insert(
            Value::String("content".to_string()),
            Value::String("Hello {{ name }}!".to_string()),
        );
        let args = Value::Mapping(map);

        assert_eq!(get_param::<String>(&args, "dest").unwrap(), "/tmp/test");

        if let Value::Mapping(args_map) = &args {
            assert!(args_map.get(Value::String("content".to_string())).is_some());
        }
    }

    #[test]
    fn rendered_template_is_idempotent_and_check_mode_does_not_write() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("rendered");
        let args = template_args("hello {{ name }}", destination.to_str().unwrap());
        let mut vars = Mapping::new();
        vars.insert(Value::String("name".into()), Value::String("world".into()));
        let args = match args {
            Value::Mapping(mut map) => {
                map.insert(Value::String("vars".into()), Value::Mapping(vars));
                Value::Mapping(map)
            }
            _ => unreachable!(),
        };
        let connection = local_connection();

        let first = execute(&connection, &args, false, "", false).unwrap();
        assert!(first.changed);
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "hello world"
        );

        let second = execute(&connection, &args, false, "", false).unwrap();
        assert!(!second.changed);

        let checked_args = template_args("changed", destination.to_str().unwrap());
        let checked = execute(&connection, &checked_args, false, "", true).unwrap();
        assert!(checked.changed);
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "hello world"
        );
    }

    #[test]
    fn template_supports_file_sources_and_dynamic_variables() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.j2");
        let destination = directory.path().join("rendered");
        std::fs::write(&source, "{{ greeting }} {{ name }}").unwrap();
        let args = serde_yaml::from_str(&format!(
            "src: {}\ndest: {}\nvars:\n  name: world\n  greeting: '{{{{ name }}}}'",
            source.display(),
            destination.display()
        ))
        .unwrap();

        let result = execute(&local_connection(), &args, false, "root", false).unwrap();
        assert!(result.changed);
        assert_eq!(std::fs::read_to_string(destination).unwrap(), "world world");
    }

    #[test]
    fn template_rejects_ambiguous_and_invalid_inputs() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("rendered");
        let cases = [
            format!("content: x\nsrc: y\ndest: {}", destination.display()),
            format!("content: 3\ndest: {}", destination.display()),
            format!("src: 3\ndest: {}", destination.display()),
            format!("src: /definitely/missing\ndest: {}", destination.display()),
            format!("dest: {}", destination.display()),
            format!("content: '{{{{ broken'\ndest: {}", destination.display()),
            format!("content: x\ndest: {}\nvars: wrong", destination.display()),
            "content: x\ndest: ''".to_string(),
        ];
        for input in cases {
            let args = serde_yaml::from_str(&input).unwrap();
            assert!(
                execute(&local_connection(), &args, false, "root", false).is_err(),
                "accepted invalid template arguments: {input}"
            );
        }
        assert!(execute(
            &local_connection(),
            &Value::String("not a mapping".to_string()),
            false,
            "root",
            false,
        )
        .is_err());
    }
}
