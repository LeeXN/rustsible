use anyhow::{anyhow, Result};
use log::debug;
use serde_yaml::Value;

use crate::inventory::Host;
use crate::modules::param::validate_params;
use crate::modules::ModuleResult;
use crate::ssh::connection::SshConnection;

/// Execute the debug module - outputs the given debug message or variable value
#[allow(dead_code)]
pub fn execute(
    _connection: &dyn SshConnection,
    args: &Value,
    _use_become: bool,
    _become_user: &str,
    _check_mode: bool,
) -> Result<ModuleResult> {
    execute_without_connection(args)
}

/// Debug is a controller-side action and must not require an SSH connection
/// merely to display an already-resolved value.
pub fn execute_without_connection(args: &Value) -> Result<ModuleResult> {
    validate_params(args, &["msg", "var", "_var_value"])?;
    debug!("Executing debug module");

    match args {
        Value::Mapping(map) => {
            // Case 1: 'msg' parameter is present - Check in two steps
            if let Some(value) = map.get(Value::String("msg".to_string())) {
                debug!("Debug module received a 'msg' parameter");
                if let Value::String(msg) = value {
                    debug!("Debug module message is {} bytes", msg.len());
                    return Ok(ModuleResult {
                        stdout: msg.clone(),
                        stderr: String::new(),
                        rc: None,
                        changed: false,
                        failed: false,
                        msg: msg.clone(),
                    });
                } else {
                    let msg_str = format_value(value);
                    return Ok(ModuleResult {
                        stdout: msg_str.clone(),
                        stderr: String::new(),
                        rc: None,
                        changed: false,
                        failed: false,
                        msg: msg_str.clone(),
                    });
                }
            } else {
                debug!("Debug module did not receive a 'msg' parameter");
            }

            // Case 2: 'var' parameter is present
            if let Some(var_param_value) = map.get(Value::String("var".to_string())) {
                debug!("Debug module received a 'var' parameter");
                // Subcase 2a: '_var_value' is also present (meaning 'var' was a variable name)
                if let Some(resolved_value) = map.get(Value::String("_var_value".to_string())) {
                    debug!("DEBUG MODULE EXECUTE: Found '_var_value' key.");
                    let value_str = format_value(resolved_value);
                    let var_name = match var_param_value {
                        Value::String(s) => s.clone(),
                        _ => "<unknown>".to_string(), // Should ideally be string
                    };
                    Ok(ModuleResult {
                        stdout: value_str.clone(),
                        stderr: String::new(),
                        rc: None,
                        changed: false,
                        failed: false,
                        msg: format!("{} = {}", var_name, value_str),
                    })
                // Subcase 2b: '_var_value' is NOT present, check if 'var' holds a rendered string
                } else if let Value::String(rendered_string) = var_param_value {
                    debug!("DEBUG MODULE EXECUTE: 'var' key holds a string.");
                    Ok(ModuleResult {
                        stdout: rendered_string.clone(),
                        stderr: String::new(),
                        rc: None,
                        changed: false,
                        failed: false,
                        msg: rendered_string.clone(),
                    })
                // Subcase 2c: 'var' exists but is not a string, and '_var_value' is missing
                } else {
                    debug!("DEBUG MODULE EXECUTE: 'var' key holds non-string value.");
                    let var_content_str = format!("{:?}", var_param_value);
                    Ok(ModuleResult {
                        stdout: var_content_str.clone(),
                        stderr: String::new(),
                        rc: None,
                        changed: false,
                        failed: false,
                        msg: var_content_str.clone(),
                    })
                }
            }
            // Case 3: Neither 'msg' nor 'var' parameter found
            else {
                debug!("DEBUG MODULE EXECUTE: Neither 'msg' nor 'var' keys found.");
                Err(anyhow!("Debug module requires a 'msg' or 'var' parameter"))
            }
        }
        _ => {
            debug!("Debug module arguments were not a mapping");
            Err(anyhow!("Debug module requires parameters as a YAML map"))
        }
    }
}

/// Execute the debug module for ad-hoc commands
pub fn execute_adhoc(
    host: &Host,
    args: &Value,
    _use_become: bool,
    _become_user: &str,
    _check_mode: bool,
) -> Result<ModuleResult> {
    debug!("Executing ad-hoc debug module for host {}", host.name);
    execute_without_connection(args)
}

/// Format YAML values in a human-readable way
fn format_value(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(), // Don't add quotes for direct variable display
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Sequence(seq) => {
            if seq.is_empty() {
                return "[]".to_string();
            }

            let mut result = String::from("\n");

            for item in seq.iter() {
                let item_str = match item {
                    Value::String(s) => format!("\"{}\"", s),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    _ => format_complex_value(item, 1),
                };

                result.push_str(&format!("  - {}\n", item_str));
            }

            result
        }
        Value::Mapping(map) => {
            if map.is_empty() {
                return "{}".to_string();
            }

            format_complex_value(value, 0)
        }
        Value::Null => "null".to_string(),
        Value::Tagged(tagged) => format_value(&tagged.value),
    }
}

/// Format complex values (maps and nested structures) with indentation
fn format_complex_value(value: &Value, indent_level: usize) -> String {
    let indent = "  ".repeat(indent_level);
    let next_indent = "  ".repeat(indent_level + 1);

    match value {
        Value::Mapping(map) => {
            if map.is_empty() {
                return "{}".to_string();
            }

            let mut result = String::from("\n");

            for (k, v) in map {
                let key_str = match k {
                    Value::String(s) => s.clone(),
                    _ => format!("{:?}", k),
                };

                let val_str = match v {
                    Value::String(s) => format!("\"{}\"", s),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    Value::Mapping(_) => format_complex_value(v, indent_level + 1),
                    Value::Sequence(_) => format_complex_value(v, indent_level + 1),
                    Value::Null => "null".to_string(),
                    Value::Tagged(tagged) => format_complex_value(&tagged.value, indent_level + 1),
                };

                // For nested structures, format with proper indentation
                if v.is_mapping() || v.is_sequence() {
                    result.push_str(&format!("{}{}:{}\n", next_indent, key_str, val_str));
                } else {
                    result.push_str(&format!("{}{}: {}\n", next_indent, key_str, val_str));
                }
            }

            result
        }
        Value::Sequence(seq) => {
            if seq.is_empty() {
                return "[]".to_string();
            }

            let mut result = String::from("\n");

            for item in seq {
                let item_str = match item {
                    Value::String(s) => format!("\"{}\"", s),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    Value::Mapping(_) => format_complex_value(item, indent_level + 1),
                    Value::Sequence(_) => format_complex_value(item, indent_level + 1),
                    Value::Null => "null".to_string(),
                    Value::Tagged(tagged) => format_complex_value(&tagged.value, indent_level + 1),
                };

                // For nested structures, format with proper indentation
                if item.is_mapping() || item.is_sequence() {
                    result.push_str(&format!("{}  -{}\n", indent, item_str));
                } else {
                    result.push_str(&format!("{}  - {}\n", indent, item_str));
                }
            }

            result
        }
        Value::Tagged(tagged) => format_complex_value(&tagged.value, indent_level),
        _ => format!("{:?}", value),
    }
}
