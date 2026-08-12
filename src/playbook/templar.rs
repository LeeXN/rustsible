use crate::playbook::filters::register_ansible_filters;
use anyhow::{anyhow, Result};
use log::{debug, warn};
use serde_yaml::Value;
use std::collections::HashMap;
use tera::{Context as TeraContext, Tera};
use uuid::Uuid;

const MAX_TEMPLATE_RECURSION: usize = 10;

/// Render a string value using Tera templating, handling recursion.
///
/// # Arguments
/// * `input` - The string potentially containing Tera expressions.
/// * `tera` - A reference to the Tera instance.
/// * `context` - The Tera context containing variables.
/// * `force_string` - A boolean indicating whether to return the result as a string.
pub fn render_value(
    input: &str,
    tera: &mut Tera,
    context: &TeraContext,
    force_string: bool,
) -> Result<Value> {
    debug!("Rendering a Tera value ({} bytes)", input.len());

    // 转换 Ansible 语法到 Tera 语法
    let converted_input = convert_ansible_to_tera_syntax(input);
    if converted_input != input {
        debug!("Converted Ansible filter syntax to Tera syntax");
    }

    // Let Tera evaluate undefined values. Filters such as `default` are
    // specifically designed to handle them; a lexical pre-check would reject
    // valid expressions before the filter gets a chance to run.

    let mut current_str = converted_input.clone();
    let mut depth = 0;

    // 多行模板渲染时，每次都新建 Tera 实例
    if force_string && converted_input.contains('\n') {
        let mut local_tera = Tera::default();
        // 注册自定义 filter
        register_ansible_filters(&mut local_tera);
        let template_name = format!("__inline_content_{}", Uuid::new_v4());
        match local_tera.add_raw_template(&template_name, &converted_input) {
            Ok(_) => {}
            Err(e) => {
                warn!("Failed to parse a multiline template");
                return Err(anyhow!("add_raw_template failed: {}", e));
            }
        }
        match local_tera.render(&template_name, context) {
            Ok(rendered) => return Ok(Value::String(rendered)),
            Err(e) => {
                warn!("Failed to render a multiline template");
                return Err(anyhow!("Tera render failed: {}", e));
            }
        }
    }

    // Loop for recursive rendering
    while depth < MAX_TEMPLATE_RECURSION {
        depth += 1;
        debug!("Rendering Tera value at recursion depth {}", depth);
        let last_str = current_str.clone();

        match tera.render_str(&current_str, context, false) {
            Ok(rendered) => {
                if rendered == last_str {
                    // No change occurred, break loop
                    debug!("Template rendering reached fixed point at depth {}", depth);
                    break;
                }
                current_str = rendered;
            }
            Err(e) => {
                warn!("Template rendering failed at recursion depth {}", depth);

                // 检查是否是变量未定义导致的 parse 错误
                let error_string = format!("{}", e);
                if error_string.contains("Failed to parse")
                    && error_string.contains("__tera_one_off")
                {
                    // 这通常意味着模板中有未定义的变量与 filter 结合
                    let undefined_vars = extract_undefined_variables(input, context);
                    if !undefined_vars.is_empty() {
                        return Err(anyhow!(
                            "Template parsing failed due to undefined variables: {}. Define these variables in the playbook.",
                            undefined_vars.join(", ")
                        ));
                    }
                }

                // 检查是否是变量未定义的错误
                if error_string.contains("Variable") && error_string.contains("not found") {
                    return Err(anyhow!("Undefined variable in template: {}", e));
                }

                // 检查是否是filter未找到的错误
                if error_string.contains("Filter") && error_string.contains("not found") {
                    return Err(anyhow!("Unknown filter in template: {}", e));
                }

                return Err(anyhow!("Tera rendering error at depth {}: {}", depth, e));
            }
        }
    }

    if depth >= MAX_TEMPLATE_RECURSION && (current_str.contains("{{") || current_str.contains("{%"))
    {
        return Err(anyhow!(
            "Template rendering did not converge within {} recursive passes",
            MAX_TEMPLATE_RECURSION
        ));
    }

    debug!(
        "Finished rendering a Tera value ({} bytes)",
        current_str.len()
    );

    if force_string {
        return Ok(Value::String(current_str));
    }

    // If the input didn't contain template syntax and the result is the same as input,
    // return as string directly without trying to parse as YAML/JSON
    if !input.contains("{{") && !input.contains("{%") && current_str == input {
        debug!("Input contains no template syntax; returning it as a string");
        return Ok(Value::String(current_str));
    }

    // Check if this looks like a single-line string that should not be parsed as YAML
    // This handles cases like "user ALL=(ALL) NOPASSWD: ALL" which contains colons but should be a string
    let should_be_string = current_str.lines().count() == 1
        && current_str.contains(' ')
        && !current_str.trim_start().starts_with('-') // Not a YAML list
        && !current_str.trim_start().starts_with('{') // Not a JSON object
        && !current_str.trim_start().starts_with('['); // Not a JSON array

    if should_be_string {
        // Test if YAML parsing would create a mapping from a single line
        if let Ok(Value::Mapping(_)) = serde_yaml::from_str(&current_str) {
            debug!(
                "Rendered single-line value resembles a YAML mapping; preserving it as a string"
            );
            return Ok(Value::String(current_str));
        }
    }

    // --- Logic to parse the final string (JSON/YAML or fallback to string) ---
    match serde_json::from_str::<serde_json::Value>(&current_str) {
        Ok(json_value) => {
            match serde_yaml::to_value(json_value) {
                Ok(yaml_value) => Ok(yaml_value),
                Err(e) => {
                    warn!("Failed to convert rendered JSON value to YAML: {}. Preserving it as a string.", e);
                    Ok(Value::String(current_str))
                }
            }
        }
        Err(_) => {
            match serde_yaml::from_str(&current_str) {
                Ok(yaml_value) => {
                    // Check if YAML parsing resulted in null for what should be a string
                    if yaml_value == Value::Null && !current_str.trim().is_empty() {
                        debug!("YAML parsing returned null for a non-empty value; preserving it as a string");
                        Ok(Value::String(current_str))
                    } else {
                        Ok(yaml_value)
                    }
                }
                Err(_) => Ok(Value::String(current_str)),
            }
        }
    }
}

/// Helper function to check for potentially undefined variables in a template
fn check_template_variables(template: &str, context: &TeraContext) -> Vec<String> {
    let mut undefined_vars = Vec::new();

    // `find` returns byte offsets that are guaranteed to be UTF-8 boundaries.
    // Keeping all slicing relative to those offsets avoids mixing character and
    // byte indexes when non-ASCII text appears before or inside an expression.
    let mut remainder = template;
    while let Some(open_index) = remainder.find("{{") {
        let after_open = &remainder[open_index + 2..];
        let Some(close_index) = after_open.find("}}") else {
            break;
        };
        let expression = after_open[..close_index].trim_start();

        if matches!(expression.chars().next(), Some('\'' | '"')) {
            debug!("Skipping variable check for a string-literal expression");
        } else if let Some(var_name) = leading_variable_name(expression) {
            // Skip common Ansible built-in variables that might not be in context.
            if [
                "inventory_hostname",
                "ansible_hostname",
                "ansible_host",
                "ansible_port",
                "ansible_ssh_user",
                "ansible_user",
                "group_names",
                "groups",
            ]
            .contains(&var_name)
            {
                debug!("Skipping check for built-in Ansible variable: {}", var_name);
            } else if !context.contains_key(var_name)
                && !undefined_vars.iter().any(|undefined| undefined == var_name)
            {
                undefined_vars.push(var_name.to_string());
            }
        }

        remainder = &after_open[close_index + 2..];
    }

    undefined_vars
}

/// Return the root variable at the beginning of a Tera expression.
///
/// Attribute/index access is checked against the root value stored in the
/// context, so `user.name` checks `user`. Literal and operator-led expressions
/// intentionally return `None` and are left to Tera's parser.
fn leading_variable_name(expression: &str) -> Option<&str> {
    let first = expression.chars().next()?;
    if !(first == '_' || first.is_alphabetic()) {
        return None;
    }

    let end = expression
        .char_indices()
        .find_map(|(index, ch)| {
            if ch == '_' || ch.is_alphanumeric() {
                None
            } else {
                Some(index)
            }
        })
        .unwrap_or(expression.len());
    let candidate = &expression[..end];

    if matches!(candidate, "true" | "false" | "none" | "null") {
        None
    } else {
        Some(candidate)
    }
}

/// Helper function to extract undefined variables from a template
fn extract_undefined_variables(template: &str, context: &TeraContext) -> Vec<String> {
    check_template_variables(template, context)
}

/// Helper function to convert a variable map to a Tera Context.
/// This should ideally happen once before processing a task or loop.
pub fn create_tera_context(vars: &HashMap<String, Value>) -> Result<TeraContext> {
    let mut context = TeraContext::new();
    for (key, value) in vars {
        // Convert serde_yaml::Value back to serde_json::Value for Tera context
        let json_val = serde_json::to_value(value).map_err(|error| {
            anyhow!(
                "Could not convert variable '{}' to the template context: {}",
                key,
                error
            )
        })?;
        context.insert(key.clone(), &json_val);
    }
    Ok(context)
}

/// Evaluate a condition expression using Tera.
///
/// # Arguments
/// * `condition` - The condition string (e.g., "item.enabled == true").
/// * `tera` - A reference to the Tera instance with registered filters.
/// * `context` - The Tera context containing variables for evaluation.
pub fn evaluate_condition(condition: &str, tera: &mut Tera, context: &TeraContext) -> Result<bool> {
    debug!("Evaluating a Tera condition ({} bytes)", condition.len());
    register_ansible_filters(tera);

    // Render the condition expression directly
    let template = format!("{{{{ {} }}}}", condition);
    match render_value(&template, tera, context, false) {
        Ok(result_value) => {
            // Evaluate truthiness of the resulting Value
            Ok(evaluate_truthiness(&result_value))
        }
        Err(e) => {
            warn!("A Tera condition could not be evaluated");
            Err(anyhow!("Tera condition evaluation error: {}", e))
        }
    }
}

// Helper function to evaluate the truthiness of a serde_yaml::Value
fn evaluate_truthiness(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i != 0
            } else if let Some(u) = n.as_u64() {
                u != 0
            } else if let Some(f) = n.as_f64() {
                f != 0.0
            } else {
                false
            } // Should not happen with standard numbers
        }
        Value::String(s) => !s.is_empty(),
        Value::Sequence(seq) => !seq.is_empty(),
        Value::Mapping(map) => !map.is_empty(),
        Value::Null => false,
        Value::Tagged(tagged) => evaluate_truthiness(&tagged.value), // Evaluate inner value for tagged types
    }
}

/// Convert Ansible filter syntax to Tera syntax
/// Converts {{ var | filter('arg') }} to {{ var | filter(arg='arg') }}
fn convert_ansible_to_tera_syntax(input: &str) -> String {
    let converted = convert_filter_calls(input, "password_hash", &["hash_type", "salt"]);
    convert_filter_calls(&converted, "selectattr", &["key", "test", "value"])
}

/// Tera requires named filter arguments while Ansible commonly uses positional
/// filter arguments. Convert the supported filters without making assumptions
/// about the actual expressions used as arguments.
fn convert_filter_calls(input: &str, filter_name: &str, argument_names: &[&str]) -> String {
    let prefix = format!("{}(", filter_name);
    let mut output = String::with_capacity(input.len());
    let mut remaining = input;

    while let Some(start) = remaining.find(&prefix) {
        output.push_str(&remaining[..start]);
        let args_start = start + prefix.len();
        let Some(close_offset) = find_call_end(&remaining[args_start..]) else {
            output.push_str(&remaining[start..]);
            return output;
        };
        let args_end = args_start + close_offset;
        let args = &remaining[args_start..args_end];
        let parts = split_filter_args(args);
        output.push_str(&prefix);
        if (1..=argument_names.len()).contains(&parts.len())
            && parts.iter().all(|part| !part.trim().is_empty())
        {
            for (index, part) in parts.iter().enumerate() {
                if index > 0 {
                    output.push_str(", ");
                }
                let part = part.trim();
                if has_top_level_assignment(part) {
                    output.push_str(part);
                } else {
                    output.push_str(argument_names[index]);
                    output.push('=');
                    output.push_str(part);
                }
            }
        } else {
            output.push_str(args);
        }
        output.push(')');
        remaining = &remaining[args_end + 1..];
    }

    output.push_str(remaining);
    output
}

fn find_call_end(input: &str) -> Option<usize> {
    let mut quote = None;
    let mut escaped = false;
    let mut nested = 0usize;
    for (index, character) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote.is_some() {
            escaped = true;
            continue;
        }
        if matches!(character, '\'' | '"') {
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            }
            continue;
        }
        if quote.is_some() {
            continue;
        }
        match character {
            '(' | '[' | '{' => nested += 1,
            ')' if nested == 0 => return Some(index),
            ')' | ']' | '}' => nested = nested.saturating_sub(1),
            _ => {}
        }
    }
    None
}

fn split_filter_args(input: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut quote = None;
    let mut escaped = false;
    let mut nested = 0usize;
    for (index, character) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote.is_some() {
            escaped = true;
            continue;
        }
        if matches!(character, '\'' | '"') {
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            }
            continue;
        }
        if quote.is_some() {
            continue;
        }
        match character {
            '(' | '[' | '{' => nested += 1,
            ')' | ']' | '}' => nested = nested.saturating_sub(1),
            ',' if nested == 0 => {
                parts.push(&input[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if !input.trim().is_empty() || !parts.is_empty() {
        parts.push(&input[start..]);
    }
    parts
}

fn has_top_level_assignment(input: &str) -> bool {
    let mut quote = None;
    let mut escaped = false;
    let mut nested = 0usize;
    for character in input.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote.is_some() {
            escaped = true;
            continue;
        }
        if matches!(character, '\'' | '"') {
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            }
            continue;
        }
        if quote.is_some() {
            continue;
        }
        match character {
            '(' | '[' | '{' => nested += 1,
            ')' | ']' | '}' => nested = nested.saturating_sub(1),
            '=' if nested == 0 => return true,
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::Mapping;

    // Helper to create Tera instance for tests
    fn create_test_tera() -> Tera {
        Tera::default()
    }

    // Helper to create Tera context from HashMap<String, Value>
    fn create_test_context_from_map(vars: &HashMap<String, Value>) -> TeraContext {
        create_tera_context(vars).unwrap()
    }

    #[test]
    fn test_render_value_simple() {
        let mut tera = create_test_tera();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), Value::String("World".to_string()));
        let context = create_test_context_from_map(&vars);

        let result = render_value("Hello {{ name }}!", &mut tera, &context, false).unwrap();
        assert_eq!(result, Value::String("Hello World!".to_string()));
    }

    #[test]
    fn template_context_rejects_values_that_json_cannot_represent() {
        let mut invalid_mapping = Mapping::new();
        invalid_mapping.insert(
            Value::Sequence(vec![Value::String("compound".to_string())]),
            Value::String("value".to_string()),
        );
        let vars = HashMap::from([("invalid".to_string(), Value::Mapping(invalid_mapping))]);

        let error = create_tera_context(&vars).unwrap_err();
        assert!(error.to_string().contains("template context"));
    }

    #[test]
    fn test_render_value_with_chinese_and_emoji_is_utf8_safe() {
        let mut tera = create_test_tera();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), Value::String("世界".to_string()));
        vars.insert("emoji".to_string(), Value::String("🚀".to_string()));
        let context = create_test_context_from_map(&vars);

        let result = render_value(
            "你好，{{ name }}！准备出发 {{ emoji }}",
            &mut tera,
            &context,
            false,
        )
        .unwrap();

        assert_eq!(result, Value::String("你好，世界！准备出发 🚀".to_string()));
    }

    #[test]
    fn test_template_variable_check_scans_multiple_utf8_expressions() {
        let mut context = TeraContext::new();
        context.insert("defined", "值😀");

        let undefined = check_template_variables(
            "前缀😀 {{ defined }} / {{ first_missing | default(value='中文') }} / {{ second_missing }} / {{ first_missing }}",
            &context,
        );

        assert_eq!(undefined, vec!["first_missing", "second_missing"]);
    }

    #[test]
    fn test_template_variable_check_continues_after_utf8_string_literal() {
        let context = TeraContext::new();

        let undefined = check_template_variables("{{ '中文😀' }} then {{ missing }}", &context);

        assert_eq!(undefined, vec!["missing"]);
    }

    #[test]
    fn test_template_variable_check_uses_root_for_attribute_access() {
        let mut context = TeraContext::new();
        context.insert("user", &serde_json::json!({ "name": "测试" }));

        let undefined = check_template_variables("👤 {{ user.name }}", &context);

        assert!(undefined.is_empty());
    }

    #[test]
    fn test_render_value_complex_expression() {
        let mut tera = create_test_tera();
        let mut vars = HashMap::new();
        vars.insert("a".to_string(), Value::Number(5.into()));
        vars.insert("b".to_string(), Value::Number(3.into()));
        let context = create_test_context_from_map(&vars);

        let result = render_value("{{ a + b * 2 }}", &mut tera, &context, false).unwrap();
        // Tera should evaluate this to 11
        assert_eq!(result, Value::Number(serde_yaml::Number::from(11)));
    }

    #[test]
    fn test_render_value_conditional_expression() {
        let mut tera = create_test_tera();
        let mut vars = HashMap::new();
        vars.insert("use_prod".to_string(), Value::Bool(true));
        let context = create_test_context_from_map(&vars);

        let result = render_value(
            "{% if use_prod %}production{% else %}staging{% endif %}",
            &mut tera,
            &context,
            false,
        )
        .unwrap();
        assert_eq!(result, Value::String("production".to_string()));

        let mut vars2 = HashMap::new();
        vars2.insert("use_prod".to_string(), Value::Bool(false));
        let context2 = create_test_context_from_map(&vars2);
        let result2 = render_value(
            "{% if use_prod %}production{% else %}staging{% endif %}",
            &mut tera,
            &context2,
            false,
        )
        .unwrap();
        assert_eq!(result2, Value::String("staging".to_string()));
    }

    #[test]
    fn test_evaluate_condition_simple_true() {
        let mut tera = create_test_tera();
        let mut vars = HashMap::new();
        vars.insert("enabled".to_string(), Value::Bool(true));
        let context = create_test_context_from_map(&vars);
        assert!(evaluate_condition("enabled", &mut tera, &context).unwrap());
    }

    #[test]
    fn test_evaluate_condition_simple_false() {
        let mut tera = create_test_tera();
        let mut vars = HashMap::new();
        vars.insert("enabled".to_string(), Value::Bool(false));
        let context = create_test_context_from_map(&vars);
        assert!(!evaluate_condition("enabled", &mut tera, &context).unwrap());
    }

    #[test]
    fn test_evaluate_condition_comparison() {
        let mut tera = create_test_tera();
        let mut vars = HashMap::new();
        vars.insert("count".to_string(), Value::Number(5.into()));
        vars.insert("name".to_string(), Value::String("test".into()));
        let context = create_test_context_from_map(&vars);
        assert!(evaluate_condition("count > 3", &mut tera, &context).unwrap());
        assert!(!evaluate_condition("count < 5", &mut tera, &context).unwrap());
        assert!(evaluate_condition("count == 5", &mut tera, &context).unwrap());
        assert!(evaluate_condition("name == \"test\"", &mut tera, &context).unwrap());
        assert!(!evaluate_condition("name != 'test'", &mut tera, &context).unwrap());
    }

    #[test]
    fn test_evaluate_condition_is_defined() {
        let mut tera = create_test_tera();
        let mut vars = HashMap::new();
        vars.insert("defined_var".to_string(), Value::String("hello".into()));
        // undefined_var is not inserted
        let context = create_test_context_from_map(&vars);
        assert!(evaluate_condition("defined_var is defined", &mut tera, &context).unwrap());
        assert!(!evaluate_condition("undefined_var is defined", &mut tera, &context).unwrap());
    }

    #[test]
    fn test_evaluate_condition_is_not_defined() {
        let mut tera = create_test_tera();
        let mut vars = HashMap::new();
        vars.insert("defined_var".to_string(), Value::String("hello".into()));
        let context = create_test_context_from_map(&vars);
        assert!(!evaluate_condition("defined_var is not defined", &mut tera, &context).unwrap());
        assert!(evaluate_condition("undefined_var is not defined", &mut tera, &context).unwrap());
    }

    #[test]
    fn test_evaluate_condition_in_list() {
        let mut tera = create_test_tera();
        let mut vars = HashMap::new();
        vars.insert(
            "my_list".to_string(),
            Value::Sequence(vec![Value::String("a".into()), Value::String("b".into())]),
        );
        vars.insert("check".to_string(), Value::String("a".into()));
        let context = create_test_context_from_map(&vars);
        assert!(evaluate_condition("'a' in my_list", &mut tera, &context).unwrap());
        assert!(!evaluate_condition("'c' in my_list", &mut tera, &context).unwrap());
        assert!(evaluate_condition("check in my_list", &mut tera, &context).unwrap());
    }

    #[test]
    fn test_evaluate_condition_complex_with_item() {
        let mut tera = create_test_tera();
        let mut item_map = Mapping::new();
        item_map.insert(Value::String("enabled".to_string()), Value::Bool(true));
        item_map.insert(Value::String("level".to_string()), Value::Number(2.into()));

        let mut vars = HashMap::new();
        vars.insert("item".to_string(), Value::Mapping(item_map));
        let context = create_test_context_from_map(&vars);

        assert!(evaluate_condition("item.enabled", &mut tera, &context).unwrap());
        assert!(evaluate_condition("item.level > 1", &mut tera, &context).unwrap());
        assert!(
            evaluate_condition("item.enabled and item.level > 1", &mut tera, &context).unwrap()
        );
        assert!(
            !evaluate_condition("item.enabled and item.level < 1", &mut tera, &context).unwrap()
        );
        assert!(evaluate_condition("item.enabled or item.level < 1", &mut tera, &context).unwrap());
    }

    #[test]
    fn test_render_value_undefined_variable_with_filter() {
        let mut tera = create_test_tera();
        // 注册 password_hash filter
        register_ansible_filters(&mut tera);

        // 创建空的上下文（不定义 root_password 变量）
        let context = TeraContext::new();

        // 测试未定义变量与 filter 的组合
        let result = render_value(
            "{{ root_password | password_hash('sha512', 'testsalt') }}",
            &mut tera,
            &context,
            false,
        );

        // 应该返回错误
        assert!(result.is_err());

        let error = result.unwrap_err();
        println!("Error: {}", error);

        // 错误应该指示变量未定义
        let error_string = format!("{}", error);
        assert!(
            error_string.contains("Variable")
                || error_string.contains("not found")
                || error_string.contains("root_password")
        );
    }

    #[test]
    fn default_filter_can_handle_an_undefined_variable() {
        let mut tera = create_test_tera();
        let context = TeraContext::new();

        let result = render_value(
            "{{ missing | default(value='fallback') }}",
            &mut tera,
            &context,
            false,
        )
        .unwrap();

        assert_eq!(result, Value::String("fallback".to_string()));
    }

    #[test]
    fn test_render_value_defined_variable_with_filter() {
        let mut tera = create_test_tera();
        // 注册 password_hash filter
        register_ansible_filters(&mut tera);

        // 创建包含定义变量的上下文
        let mut vars = HashMap::new();
        vars.insert(
            "test_password".to_string(),
            Value::String("mypassword123".to_string()),
        );
        let context = create_test_context_from_map(&vars);

        // 测试已定义变量与 filter 的组合
        let result = render_value(
            "{{ test_password | password_hash('sha512', 'testsalt') }}",
            &mut tera,
            &context,
            false,
        );

        // 打印结果以便调试
        match &result {
            Ok(rendered) => {
                println!("Success: {:?}", rendered);
            }
            Err(e) => {
                println!("Error: {}", e);
            }
        }

        // 应该成功
        assert!(result.is_ok());

        let rendered = result.unwrap();
        if let Value::String(hash) = rendered {
            println!("Generated hash: {}", hash);
            // SHA-512 哈希应该以 $6$ 开头
            assert!(hash.starts_with("$6$"));
        } else {
            panic!("Expected string result");
        }
    }

    #[test]
    fn test_simple_tera_rendering() {
        let mut tera = create_test_tera();

        // 创建包含定义变量的上下文
        let mut vars = HashMap::new();
        vars.insert(
            "test_password".to_string(),
            Value::String("mypassword123".to_string()),
        );
        let context = create_test_context_from_map(&vars);

        // 测试简单的变量渲染（不使用 filter）
        let result = render_value("{{ test_password }}", &mut tera, &context, false);
        println!("Simple variable result: {:?}", result);
        assert!(result.is_ok());

        // 测试 Tera 的 render_str 直接调用
        let direct_result = tera.render_str("{{ test_password }}", &context, false);
        println!("Direct Tera result: {:?}", direct_result);
        assert!(direct_result.is_ok());

        // 注册 filter 并测试
        register_ansible_filters(&mut tera);

        // 测试 filter 是否注册成功
        let filter_result = render_value(
            "{{ test_password | password_hash('sha512', 'testsalt') }}",
            &mut tera,
            &context,
            false,
        );
        assert!(filter_result.is_ok());

        // 尝试不同的语法
        let filter_result2 = tera.render_str(
            "{{ test_password | password_hash(hash_type='sha512', salt='testsalt') }}",
            &context,
            false,
        );
        assert!(filter_result2.is_ok());

        // 尝试没有引号的语法
        let filter_result3 = tera.render_str(
            "{{ test_password | password_hash(hash_type='sha512') }}",
            &context,
            false,
        );
        assert!(filter_result3.is_err());
    }

    #[test]
    fn converts_ansible_password_hash_positional_arguments() {
        assert_eq!(
            convert_ansible_to_tera_syntax(
                "{{ secret | password_hash('sha512', inventory_hostname) }}"
            ),
            "{{ secret | password_hash(hash_type='sha512', salt=inventory_hostname) }}"
        );
        assert_eq!(
            convert_ansible_to_tera_syntax(
                "{{ secret | password_hash(hash_type='sha256', salt='stable') }}"
            ),
            "{{ secret | password_hash(hash_type='sha256', salt='stable') }}"
        );
        assert_eq!(
            convert_ansible_to_tera_syntax("{{ users | selectattr('enabled', 'equalto', true) }}"),
            "{{ users | selectattr(key='enabled', test='equalto', value=true) }}"
        );
    }

    #[test]
    fn tera_string_iteration_preserves_grapheme_clusters() {
        let tera = Tera::default();
        let mut context = TeraContext::new();
        context.insert("text", "👨‍👩‍👧‍👦a");

        let rendered = tera
            .render_str(
                "{% for character in text %}{{ character }}|{% endfor %}",
                &context,
                false,
            )
            .unwrap();
        assert_eq!(rendered, "👨‍👩‍👧‍👦|a|");
    }
}
