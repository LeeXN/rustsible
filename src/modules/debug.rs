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
                        values: Default::default(),
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
                        values: Default::default(),
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
                        values: Default::default(),
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
                        values: Default::default(),
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
                        values: Default::default(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::connection::MockSshConnection;

    fn yaml(input: &str) -> Value {
        serde_yaml::from_str(input).unwrap()
    }

    #[test]
    fn debug_formats_messages_and_resolved_variables() {
        for (input, stdout, message) in [
            ("msg: hello", "hello", "hello"),
            ("msg: 42", "42", "42"),
            ("var: rendered", "rendered", "rendered"),
            ("var: 7", "Number(7)", "Number(7)"),
            (
                "var: inventory_hostname\n_var_value: [web, 2, true]",
                "\n  - \"web\"\n  - 2\n  - true\n",
                "inventory_hostname = \n  - \"web\"\n  - 2\n  - true\n",
            ),
            ("var: 9\n_var_value: false", "false", "<unknown> = false"),
        ] {
            let result = execute_without_connection(&yaml(input)).unwrap();
            assert_eq!(result.stdout, stdout);
            assert_eq!(result.msg, message);
            assert!(!result.changed);
        }
    }

    #[test]
    fn debug_formats_nested_yaml_values() {
        let value = yaml(
            "root:\n  text: hello\n  number: 3\n  enabled: true\n  nothing: null\n  nested:\n    - child\n    - [1, false]\nempty_map: {}\nempty_list: []",
        );
        let formatted = format_value(&value);
        for fragment in [
            "root:",
            "text: \"hello\"",
            "number: 3",
            "enabled: true",
            "nothing: null",
            "nested:",
            "empty_map:",
            "empty_list:",
        ] {
            assert!(
                formatted.contains(fragment),
                "missing {fragment}: {formatted}"
            );
        }
        assert_eq!(format_value(&Value::Sequence(vec![])), "[]");
        assert_eq!(format_value(&Value::Mapping(Default::default())), "{}");
        assert_eq!(format_value(&Value::Null), "null");
    }

    #[test]
    fn debug_validates_arguments_and_never_needs_a_connection() {
        assert!(execute_without_connection(&yaml("{}"))
            .unwrap_err()
            .to_string()
            .contains("requires a 'msg' or 'var'"));
        assert!(execute_without_connection(&Value::String("no map".to_string())).is_err());
        assert!(execute_without_connection(&yaml("msg: ok\nunknown: true")).is_err());

        let mut connection = MockSshConnection::new();
        connection.expect_execute_command().never();
        let direct = execute(&connection, &yaml("msg: direct"), true, "root", true).unwrap();
        assert_eq!(direct.stdout, "direct");
        let adhoc = execute_adhoc(
            &Host::new("unreachable.invalid"),
            &yaml("msg: adhoc"),
            false,
            "root",
            false,
        )
        .unwrap();
        assert_eq!(adhoc.stdout, "adhoc");
    }
}
