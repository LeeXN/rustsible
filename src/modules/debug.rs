use anyhow::{Result, anyhow};
use serde_yaml::Value;
use log::{info, warn, debug};

use crate::modules::ModuleResult;
use crate::ssh::connection::SshClient;
use crate::inventory::Host;

/// Execute the debug module - outputs the given debug message or variable value
#[allow(dead_code)]
pub fn execute(
    _ssh_client: &SshClient, 
    args: &Value, 
    _use_become: bool, 
    _become_user: &str
) -> Result<ModuleResult> {
    // Add detailed logging
    debug!("DEBUG MODULE EXECUTE: Received args: {:#?}", args);

    match args {
        Value::Mapping(map) => {
            debug!("DEBUG MODULE EXECUTE: Args map content: {:#?}", map);

            // Case 1: 'msg' parameter is present - Check in two steps
            if let Some(value) = map.get(&Value::String("msg".to_string())) {
                debug!("DEBUG MODULE EXECUTE: Found 'msg' key with value: {:?}", value);
                if let Value::String(msg) = value {
                     debug!("DEBUG MODULE EXECUTE: 'msg' value is a String.");
                     info!("Debug message: {}", msg);
                     return Ok(ModuleResult {
                         stdout: msg.clone(),
                         stderr: String::new(),
                         changed: false,
                         msg: msg.clone(),
                     });
                 } else {
                    let msg_str = format_value(value);
                    return Ok(ModuleResult {
                        stdout: msg_str.clone(),
                        stderr: String::new(),
                        changed: false,
                        msg: msg_str.clone(),
                    });
                 }
            } else {
                debug!("DEBUG MODULE EXECUTE: Did not find 'msg' key.");
            }

            // Case 2: 'var' parameter is present
            if let Some(var_param_value) = map.get(&Value::String("var".to_string())) {
                debug!("DEBUG MODULE EXECUTE: Found 'var' key.");
                // Subcase 2a: '_var_value' is also present (meaning 'var' was a variable name)
                if let Some(resolved_value) = map.get(&Value::String("_var_value".to_string())) {
                    debug!("DEBUG MODULE EXECUTE: Found '_var_value' key.");
                    let value_str = format_value(resolved_value);
                    let var_name = match var_param_value {
                        Value::String(s) => s.clone(),
                        _ => "<unknown>".to_string(), // Should ideally be string
                    };
                    info!("Debug var '{}': {}", var_name, value_str);
                    return Ok(ModuleResult {
                        stdout: value_str.clone(),
                        stderr: String::new(),
                        changed: false,
                        msg: format!("{} = {}", var_name, value_str),
                    });
                // Subcase 2b: '_var_value' is NOT present, check if 'var' holds a rendered string
                } else if let Value::String(rendered_string) = var_param_value {
                    debug!("DEBUG MODULE EXECUTE: 'var' key holds a string.");
                    info!("Debug var (direct string): {}", rendered_string);
                    return Ok(ModuleResult {
                        stdout: rendered_string.clone(),
                        stderr: String::new(),
                        changed: false,
                        msg: rendered_string.clone(),
                    });
                // Subcase 2c: 'var' exists but is not a string, and '_var_value' is missing
                } else {
                    debug!("DEBUG MODULE EXECUTE: 'var' key holds non-string value.");
                    let var_content_str = format!("{:?}", var_param_value);
                    return Ok(ModuleResult {
                        stdout: var_content_str.clone(),
                        stderr: String::new(),
                        changed: false,
                        msg: var_content_str.clone(),
                    });
                }
            }

            // Case 3: Neither 'msg' nor 'var' parameter found
            else {
                debug!("DEBUG MODULE EXECUTE: Neither 'msg' nor 'var' keys found.");
                Err(anyhow!("Debug module requires a 'msg' or 'var' parameter"))
            }
        },
        _ => {
             debug!("DEBUG MODULE EXECUTE: Received args were not a Mapping: {:?}", args);
             Err(anyhow!("Debug module requires parameters as a YAML map"))
        }
    }
}

/// Execute the debug module for ad-hoc commands
pub fn execute_adhoc(host: &Host, args: &Value) -> Result<ModuleResult> {
    info!("Debug [{}]: {:?}", host.name, args);
    match args {
        Value::Mapping(map) => {
             // Case 1: 'msg' parameter is present
            if let Some(Value::String(msg)) = map.get(&Value::String("msg".to_string())) {
                info!("Debug: {}", msg);
                return Ok(ModuleResult {
                    stdout: msg.clone(),
                    stderr: String::new(),
                    changed: false,
                    msg: msg.clone(),
                });
            }

            // Case 2: 'var' parameter is present
            if let Some(var_param_value) = map.get(&Value::String("var".to_string())) {
                // Subcase 2a: '_var_value' is also present (meaning 'var' was a variable name)
                if let Some(resolved_value) = map.get(&Value::String("_var_value".to_string())) {
                    let value_str = format_value(resolved_value);
                     let var_name = match var_param_value {
                        Value::String(s) => s.clone(),
                        _ => "<unknown>".to_string(), // Should ideally be string
                    };
                    info!("Debug var '{}': {}", var_name, value_str);
                    return Ok(ModuleResult {
                        stdout: value_str.clone(),
                        stderr: String::new(),
                        changed: false,
                        msg: format!("{} = {}", var_name, value_str),
                    });
                 // Subcase 2b: '_var_value' is NOT present, check if 'var' holds a rendered string
                } else if let Value::String(rendered_string) = var_param_value {
                     info!("Debug var (direct string): {}", rendered_string);
                     return Ok(ModuleResult {
                        stdout: rendered_string.clone(),
                        stderr: String::new(),
                        changed: false,
                        msg: rendered_string.clone(),
                    });
                // Subcase 2c: 'var' exists but is not a string, and '_var_value' is missing
                } else {
                    let var_content_str = format!("{:?}", var_param_value);
                    warn!("Debug module found 'var' parameter, but its value is not a string and _var_value is missing: {}", var_content_str);
                    return Err(anyhow!("Invalid value type for 'var' parameter in debug module: {}", var_content_str));
                }
            }

            // Case 3: Neither 'msg' nor 'var' parameter found
            Err(anyhow!("Debug module requires a 'msg' or 'var' parameter"))
        },
        _ => Err(anyhow!("Debug module requires parameters")),
    }
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
        },
        Value::Mapping(map) => {
            if map.is_empty() {
                return "{}".to_string();
            }
            
            format_complex_value(value, 0)
        },
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
        },
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
        },
        Value::Tagged(tagged) => format_complex_value(&tagged.value, indent_level),
        _ => format!("{:?}", value),
    }
} 