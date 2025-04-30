use anyhow::{Result, Context};
use log::{info, warn, debug, error};
use serde_yaml::Value;
use std::fs;
use std::path::Path;
use std::error::Error;
use tera::{Tera, Context as TeraContext};
use serde_json;

use crate::inventory::Host;
use crate::ssh::connection::SshClient;
use crate::modules::ModuleResult;
use crate::modules::param::{get_param, get_optional_param};
use crate::playbook::filters::register_ansible_filters;

/// Execute the template module logic: render and upload a template, set permissions/ownership if needed.
pub fn execute(ssh_client: &SshClient, template_args: &Value, use_become: bool, _become_user: &str) -> Result<ModuleResult> {
    let dest = get_param::<String>(template_args, "dest")?;
    
    // Extract optional parameters
    let mode = get_optional_param::<String>(template_args, "mode");
    let owner = get_optional_param::<String>(template_args, "owner");
    let group = get_optional_param::<String>(template_args, "group");
    
    // 解析模板内容 - 可以来自文件或直接内容
    let template_string: String;
    let src_display: String; // 用于日志和消息
    
    // 检查是否提供了内联内容
    if let Value::Mapping(args_map) = template_args {
        if let Some(content_value) = args_map.get(&Value::String("content".to_string())) {
            template_string = match content_value {
                Value::String(s) => s.clone(),
                _ => format!("{:?}", content_value),
            };
            src_display = "<inline_template>".to_string();
        } else if let Some(Value::String(src)) = args_map.get(&Value::String("src".to_string())) {
            // 从文件读取模板内容
            let src_path = Path::new(src);
            if !src_path.exists() {
                return Err(anyhow::anyhow!("Template file does not exist: {}", src));
            }
            template_string = fs::read_to_string(src)
                .with_context(|| format!("Failed to read template file: {}", src))?;
            src_display = src.clone();
        } else {
            return Err(anyhow::anyhow!("Template requires either 'src' or 'content' parameter"));
        }
    } else {
        return Err(anyhow::anyhow!("Template arguments must be a mapping"));
    }
    
    info!("Rendering template: {} -> {}", src_display, dest);
    
    // 创建 Tera 上下文
    let mut tera_context = TeraContext::new();
    
    // Extract vars parameter if present and convert ALL variables to Tera context
    if let Value::Mapping(args_map) = template_args {
        if let Some(vars_value) = args_map.get(&Value::String("vars".to_string())) {
            debug!("Found 'vars' parameter with {} items", 
                if let Value::Mapping(m) = vars_value { m.len() } else { 0 });
                
            if let Value::Mapping(vars_map) = vars_value {
                // 首先创建一个临时的Tera上下文用于预渲染变量
                let mut temp_tera = Tera::default();
                let mut temp_context = TeraContext::new();
                
                // 先添加所有简单变量到临时上下文
                for (key, value) in vars_map {
                    if let Value::String(key_str) = key {
                        match value {
                            Value::String(s) if !s.contains("{{") && !s.contains("{%") => {
                                // 不包含模板表达式的字符串直接添加
                                match serde_json::to_value(value) {
                                    Ok(json_val) => {
                                        temp_context.insert(key_str, &json_val);
                                        tera_context.insert(key_str, &json_val);
                                        debug!("Added simple variable '{}' to Tera context", key_str);
                                    },
                                    Err(e) => {
                                        warn!("Failed to convert simple variable '{}' to JSON: {}", key_str, e);
                                    }
                                }
                            },
                            Value::Number(_) | Value::Bool(_) => {
                                // 数字和布尔值直接添加
                                match serde_json::to_value(value) {
                                    Ok(json_val) => {
                                        temp_context.insert(key_str, &json_val);
                                        tera_context.insert(key_str, &json_val);
                                        debug!("Added primitive variable '{}' to Tera context", key_str);
                                    },
                                    Err(e) => {
                                        warn!("Failed to convert primitive variable '{}' to JSON: {}", key_str, e);
                                    }
                                }
                            },
                            Value::Mapping(_) | Value::Sequence(_) => {
                                // 复杂对象直接添加（不需要预渲染）
                                match serde_json::to_value(value) {
                                    Ok(json_val) => {
                                        temp_context.insert(key_str, &json_val);
                                        tera_context.insert(key_str, &json_val);
                                        debug!("Added complex variable '{}' to Tera context", key_str);
                                    },
                                    Err(e) => {
                                        warn!("Failed to convert complex variable '{}' to JSON: {}", key_str, e);
                                    }
                                }
                            },
                            _ => {
                                debug!("Processing template variable '{}' (will handle in second pass)", key_str);
                            }
                        }
                    }
                }
                
                // 第二遍：处理包含模板表达式的字符串变量
                for (key, value) in vars_map {
                    if let Value::String(key_str) = key {
                        if let Value::String(s) = value {
                            if s.contains("{{") || s.contains("{%") {
                                debug!("Rendering template variable '{}': {}", key_str, s);
                                
                                // 尝试渲染包含模板表达式的字符串
                                match temp_tera.render_str(s, &temp_context) {
                                    Ok(rendered_value) => {
                                        debug!("Successfully pre-rendered variable '{}': {}", key_str, rendered_value);
                                        tera_context.insert(key_str, &rendered_value);
                                        
                                        // 也更新临时上下文，以供后续变量使用
                                        temp_context.insert(key_str, &rendered_value);
                                    },
                                    Err(e) => {
                                        warn!("Failed to pre-render template variable '{}': {}. Using original value.", key_str, e);
                                        // 如果渲染失败，使用原始值
                                        tera_context.insert(key_str, s);
                                        temp_context.insert(key_str, s);
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                warn!("'vars' parameter is not a mapping: {:?}", vars_value);
            }
        } else {
            debug!("No 'vars' parameter found in template arguments");
        }
    }
    
    debug!("Final Tera context has been populated with variables");
    
    // Render the template
    let mut tera = Tera::default();
    register_ansible_filters(&mut tera);

    let rendered_content = tera.render_str(&template_string, &tera_context)
        .map_err(|e| {
            error!("Template rendering failed: {}", e);
            error!("Template content: {}", template_string);
            debug!("Template rendering context variables were available");
            
            // Try to provide more detailed error information
            let error_msg = format!("Template rendering failed: {}", e);
            if let Some(source) = e.source() {
                anyhow::anyhow!("{}\nCaused by: {}", error_msg, source)
            } else {
                anyhow::anyhow!(error_msg)
            }
        })?;
    
    info!("Template rendered successfully, uploading to remote host{}", if use_become { " (with sudo)" } else { "" });
    
    // Write the rendered template to the destination using appropriate method
    if use_become {
        // Use sudo-aware file writing method
        ssh_client.write_file_with_sudo(
            &rendered_content, 
            &dest, 
            mode.as_deref(), 
            owner.as_deref(), 
            group.as_deref()
        )?;
    } else {
        // Write file normally
        ssh_client.write_file_content(&dest, &rendered_content)?;
        
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
    
    info!("Template rendered and uploaded successfully");
    Ok(ModuleResult {
        stdout: String::new(),
        stderr: String::new(),
        changed: true,
        msg: format!("Template {} applied to {}", src_display, dest),
    })
}

/// Execute the template module in ad-hoc mode for a single host.
pub fn execute_adhoc(host: &Host, template_args: &Value) -> Result<ModuleResult> {
    info!("Connecting to host: {}", host.name);
    let ssh_client = SshClient::connect(host)?;
    
    // 解析目标文件路径
    let dest_file = get_param::<String>(template_args, "dest")?;
    
    // 检查是否使用 src 或 content
    let src_display = if let Value::Mapping(args_map) = template_args {
        if let Some(Value::String(_)) = args_map.get(&Value::String("content".to_string())) {
            "<inline_template>".to_string()
        } else if let Some(Value::String(src)) = args_map.get(&Value::String("src".to_string())) {
            src.clone()
        } else {
            return Err(anyhow::anyhow!("Template requires either 'src' or 'content' parameter"));
        }
    } else {
        return Err(anyhow::anyhow!("Template arguments must be a mapping"));
    };
    
    execute(&ssh_client, template_args, false, "")?;
    
    Ok(ModuleResult {
        stdout: String::new(),
        stderr: String::new(),
        changed: true,
        msg: format!("Template {} applied to {}", src_display, dest_file),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::{Value, Mapping};

    #[test]
    fn test_template_params() {
        let mut map = Mapping::new();
        map.insert(Value::String("dest".to_string()), Value::String("/tmp/test".to_string()));
        map.insert(Value::String("content".to_string()), Value::String("Hello {{ name }}!".to_string()));
        let args = Value::Mapping(map);
        
        assert_eq!(get_param::<String>(&args, "dest").unwrap(), "/tmp/test");
        
        if let Value::Mapping(args_map) = &args {
            assert!(args_map.get(&Value::String("content".to_string())).is_some());
        }
    }
} 