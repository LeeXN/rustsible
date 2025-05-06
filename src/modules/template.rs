use anyhow::{Result, Context};
use log::{info, warn};
use serde_yaml::Value;
use std::fs;
use std::path::Path;
use tempfile::NamedTempFile;
use tera::{Tera, Context as TeraContext};
use chrono;
use serde_yaml::Mapping;
use serde_json;

use crate::inventory::Host;
use crate::ssh::connection::SshClient;
use crate::modules::ModuleResult;
use crate::modules::param::{get_param, get_optional_param};
use crate::modules::remote::{set_file_mode, set_ownership};

/// Execute the template module logic: render and upload a template, set permissions/ownership if needed.
pub fn execute(ssh_client: &SshClient, template_args: &Value, use_become: bool, become_user: &str) -> Result<()> {
    let src = get_param::<String>(template_args, "src")?;
    let dest = get_param::<String>(template_args, "dest")?;
    
    // Extract template variables
    let mut vars = TeraContext::new();
    if let Value::Mapping(map) = template_args {
        if let Some(Value::Mapping(var_map)) = map.get(&Value::String("vars".to_string())) {
            for (key, value) in var_map {
                if let Value::String(key_str) = key {
                    match value {
                        Value::String(val_str) => { vars.insert(key_str, val_str); },
                        Value::Number(val_num) => { 
                            if val_num.is_i64() {
                                vars.insert(key_str, &val_num.as_i64().unwrap());
                            } else if val_num.is_f64() {
                                vars.insert(key_str, &val_num.as_f64().unwrap());
                            }
                        },
                        Value::Bool(val_bool) => { vars.insert(key_str, val_bool); },
                        _ => {},
                    }
                }
            }
        }
    }
    
    // Extract mode parameter (optional)
    let mode = get_optional_param::<String>(template_args, "mode");
    
    // Extract owner parameter (optional)
    let owner = get_optional_param::<String>(template_args, "owner");
    
    // Extract group parameter (optional)
    let group = get_optional_param::<String>(template_args, "group");
    
    info!("Processing template: {} -> {}", src, dest);
    
    // Check if source template file exists
    let src_path = Path::new(&src);
    if !src_path.exists() {
        return Err(anyhow::anyhow!("Template file does not exist: {}", src));
    }
    
    // Add Ansible-like facts
    // Add ansible_date_time if not already present
    if !vars.contains_key("ansible_date_time") {
        let now = chrono::Local::now();
        let mut date_time_mapping = Mapping::new();
        
        date_time_mapping.insert(
            Value::String("date".to_string()),
            Value::String(now.format("%Y-%m-%d").to_string())
        );
        date_time_mapping.insert(
            Value::String("time".to_string()),
            Value::String(now.format("%H:%M:%S").to_string())
        );
        date_time_mapping.insert(
            Value::String("year".to_string()),
            Value::String(now.format("%Y").to_string())
        );
        date_time_mapping.insert(
            Value::String("month".to_string()),
            Value::String(now.format("%m").to_string())
        );
        date_time_mapping.insert(
            Value::String("day".to_string()),
            Value::String(now.format("%d").to_string())
        );
        date_time_mapping.insert(
            Value::String("hour".to_string()),
            Value::String(now.format("%H").to_string())
        );
        date_time_mapping.insert(
            Value::String("minute".to_string()),
            Value::String(now.format("%M").to_string())
        );
        date_time_mapping.insert(
            Value::String("second".to_string()),
            Value::String(now.format("%S").to_string())
        );
        date_time_mapping.insert(
            Value::String("weekday".to_string()),
            Value::String(now.format("%A").to_string())
        );
        date_time_mapping.insert(
            Value::String("weekday_short".to_string()),
            Value::String(now.format("%a").to_string())
        );
        date_time_mapping.insert(
            Value::String("epoch".to_string()),
            Value::String(now.timestamp().to_string())
        );
        date_time_mapping.insert(
            Value::String("iso8601".to_string()),
            Value::String(now.to_rfc3339())
        );
        
        // Create value and convert to JSON for inserting into TeraContext
        let date_time_value = Value::Mapping(date_time_mapping);
        // Convert to JSON first as TeraContext expects JSON serializable values
        match serde_json::to_value(&date_time_value) {
            Ok(json_val) => {
                vars.insert("ansible_date_time", &json_val);
            },
            Err(e) => {
                warn!("Could not convert ansible_date_time to JSON: {}", e);
            }
        }
    }
    
    // Read template content
    let template_content = fs::read_to_string(src_path).context("Failed to read template file")?;
    
    // Render the template
    let mut tera = Tera::default();
    tera.add_raw_template("template", &template_content).context("Failed to add template to Tera")?;
    let rendered = tera.render("template", &vars).context("Failed to render template")?;
    
    // Create temporary file with rendered content
    let mut temp_file = NamedTempFile::new().context("Failed to create temporary file")?;
    std::io::Write::write_all(&mut temp_file, rendered.as_bytes()).context("Failed to write to temporary file")?;
    
    // Upload the rendered template
    ssh_client.upload_file(temp_file.path().to_str().unwrap(), &dest)
        .context(format!("Failed to upload rendered template to {}", dest))?;
    
    // Set file mode if specified
    if let Some(mode_str) = mode.as_deref() {
        set_file_mode(ssh_client, &dest, mode_str, use_become, become_user)?;
    }
    
    // Set ownership if specified
    if owner.is_some() || group.is_some() {
        set_ownership(ssh_client, &dest, owner.as_deref(), group.as_deref(), use_become, become_user)?;
    }
    
    info!("Template rendered and uploaded successfully");
    Ok(())
}

/// Execute the template module in ad-hoc mode for a single host.
pub fn execute_adhoc(host: &Host, template_args: &Value) -> Result<ModuleResult> {
    info!("Connecting to host: {}", host.name);
    let ssh_client = SshClient::connect(host)?;
    
    // 解析模板源文件和目标文件
    let src_file = get_param::<String>(template_args, "src")?;
    let dest_file = get_param::<String>(template_args, "dest")?;
    
    execute(&ssh_client, template_args, false, "")?;
    
    Ok(ModuleResult {
        stdout: String::new(),
        stderr: String::new(),
        changed: true,
        msg: format!("Template {} applied to {}", src_file, dest_file),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::{Value, Mapping};

    #[test]
    fn test_template_param_extract_ok() {
        let mut map = Mapping::new();
        map.insert(Value::String("src".to_string()), Value::String("/tmp/tmpl".to_string()));
        map.insert(Value::String("dest".to_string()), Value::String("/tmp/out".to_string()));
        let args = Value::Mapping(map);
        assert_eq!(crate::modules::param::get_param::<String>(&args, "src").unwrap(), "/tmp/tmpl");
        assert_eq!(crate::modules::param::get_param::<String>(&args, "dest").unwrap(), "/tmp/out");
    }

    #[test]
    fn test_template_param_missing() {
        let map = Mapping::new();
        let args = Value::Mapping(map);
        assert!(crate::modules::param::get_param::<String>(&args, "src").is_err());
    }
} 