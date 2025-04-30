use anyhow::{Result, anyhow};
use log::{debug, warn};
use std::collections::HashMap;
use serde_yaml::Value;
use tera::{Tera, Context as TeraContext};
use uuid::Uuid;
use std::error::Error;
use crate::playbook::filters::register_ansible_filters;

const MAX_TEMPLATE_RECURSION: usize = 10;

/// Render a string value using Tera templating, handling recursion.
///
/// # Arguments
/// * `input` - The string potentially containing Tera expressions.
/// * `tera` - A reference to the Tera instance.
/// * `context` - The Tera context containing variables.
/// * `force_string` - A boolean indicating whether to return the result as a string.
pub fn render_value(input: &str, tera: &mut Tera, context: &TeraContext, force_string: bool) -> Result<Value> {
    debug!("Rendering value with Tera (initial): {}", input);
    
    // 转换 Ansible 语法到 Tera 语法
    let converted_input = convert_ansible_to_tera_syntax(input);
    if converted_input != input {
        debug!("Converted Ansible syntax to Tera syntax: {} -> {}", input, converted_input);
    }
    
    // 预检查：如果模板包含变量，检查这些变量是否在上下文中存在
    if converted_input.contains("{{") {
        let potential_issues = check_template_variables(&converted_input, context);
        if !potential_issues.is_empty() {
            warn!("Template '{}' contains potentially undefined variables: {:?}", converted_input, potential_issues);
            
            // 对于包含 filter 的未定义变量，提供更好的错误信息
            for var_name in &potential_issues {
                if converted_input.contains(&format!("{} |", var_name)) {
                    return Err(anyhow!(
                        "Variable '{}' is not defined but used with a filter in template '{}'. Please define this variable in your playbook vars section.", 
                        var_name, input
                    ));
                }
            }
        }
    }
    
    let mut current_str = converted_input.clone();
    let mut depth = 0;

    // 多行模板渲染时，每次都新建 Tera 实例
    if force_string && converted_input.contains('\n') {
        let mut local_tera = Tera::default();
        // 注册自定义 filter
        register_ansible_filters(&mut local_tera);
        let template_name = format!("__inline_content_{}", Uuid::new_v4());
        match local_tera.add_raw_template(&template_name, &converted_input) {
            Ok(_) => {},
            Err(e) => {
                warn!("add_raw_template failed: {} (template_name: {})", e, template_name);
                if let Some(source) = e.source() {
                    warn!("add_raw_template error source: {}", source);
                }
                return Err(anyhow!("add_raw_template failed: {}", e));
            }
        }
        match local_tera.render(&template_name, context) {
            Ok(rendered) => return Ok(Value::String(rendered)),
            Err(e) => {
                warn!("Tera render failed: {} (template_name: {})", e, template_name);
                if let Some(source) = e.source() {
                    warn!("Tera render error source: {}", source);
                }
                return Err(anyhow!("Tera render failed: {}", e));
            }
        }
    }

    // Loop for recursive rendering
    while depth < MAX_TEMPLATE_RECURSION {
        depth += 1;
        debug!("Rendering value (depth {}): {}", depth, current_str);
        let last_str = current_str.clone();

        match tera.render_str(&current_str, context) {
            Ok(rendered) => {
                 if rendered == last_str { // No change occurred, break loop
                     debug!("Template rendering reached fixed point at depth {}", depth);
                     break;
                 }
                current_str = rendered;
            },
            Err(e) => {
                warn!("Error rendering Tera template '{}' at depth {}: {}", input, depth, e);
                
                // 提供更详细的错误信息
                if let Some(source) = e.source() {
                    warn!("Tera render error source: {}", source);
                }
                
                // 检查是否是变量未定义导致的 parse 错误
                let error_string = format!("{}", e);
                if error_string.contains("Failed to parse") && error_string.contains("__tera_one_off") {
                    // 这通常意味着模板中有未定义的变量与 filter 结合
                    let undefined_vars = extract_undefined_variables(input, context);
                    if !undefined_vars.is_empty() {
                        return Err(anyhow!(
                            "Template parsing failed due to undefined variables: {}. Template: '{}'. Please define these variables in your playbook.", 
                            undefined_vars.join(", "), input
                        ));
                    }
                }
                
                // 检查是否是变量未定义的错误
                if error_string.contains("Variable") && error_string.contains("not found") {
                    return Err(anyhow!("Undefined variable in template '{}': {}", input, e));
                }
                
                // 检查是否是filter未找到的错误
                if error_string.contains("Filter") && error_string.contains("not found") {
                    return Err(anyhow!("Unknown filter in template '{}': {}", input, e));
                }
                
                return Err(anyhow!("Tera rendering error for '{}' at depth {}: {}", input, depth, e));
            }
        }
    }

    if depth >= MAX_TEMPLATE_RECURSION {
        warn!(
            "Template rendering exceeded maximum recursion depth ({}) for input: {}. Final string: {}",
            MAX_TEMPLATE_RECURSION, input, current_str
        );
    }

    debug!("Final rendered string after recursion: {}", current_str);

    if force_string {
        return Ok(Value::String(current_str));
    }

    // If the input didn't contain template syntax and the result is the same as input,
    // return as string directly without trying to parse as YAML/JSON
    if !input.contains("{{") && !input.contains("{%") && current_str == input {
        debug!("Input contains no template syntax and unchanged, returning as string: {}", current_str);
        return Ok(Value::String(current_str));
    }

    // Check if this looks like a single-line string that should not be parsed as YAML
    // This handles cases like "user ALL=(ALL) NOPASSWD: ALL" which contains colons but should be a string
    let should_be_string = current_str.lines().count() == 1 && 
                          current_str.contains(' ') && 
                          !current_str.trim_start().starts_with('-') &&  // Not a YAML list
                          !current_str.trim_start().starts_with('{') &&  // Not a JSON object
                          !current_str.trim_start().starts_with('[');    // Not a JSON array
    
    if should_be_string {
        // Test if YAML parsing would create a mapping from a single line
        if let Ok(Value::Mapping(_)) = serde_yaml::from_str(&current_str) {
            debug!("Single-line string '{}' would be parsed as YAML mapping, returning as string instead", current_str);
            return Ok(Value::String(current_str));
        }
    }

    // --- Logic to parse the final string (JSON/YAML or fallback to string) ---
    match serde_json::from_str::<serde_json::Value>(&current_str) {
        Ok(json_value) => {
            match serde_yaml::to_value(json_value) {
                Ok(yaml_value) => Ok(yaml_value),
                Err(e) => {
                    warn!("Failed to convert final rendered JSON value '{}' to YAML: {}. Using string.", current_str, e);
                    Ok(Value::String(current_str))
                }
            }
        },
        Err(_) => {
            match serde_yaml::from_str(&current_str) {
                Ok(yaml_value) => {
                    // Check if YAML parsing resulted in null for what should be a string
                    if yaml_value == Value::Null && !current_str.trim().is_empty() {
                        debug!("YAML parsing returned null for non-empty string '{}', returning as string", current_str);
                        Ok(Value::String(current_str))
                    } else {
                        Ok(yaml_value)
                    }
                },
                Err(_) => {
                    Ok(Value::String(current_str))
                }
            }
        }
    }
}

/// Helper function to check for potentially undefined variables in a template
fn check_template_variables(template: &str, context: &TeraContext) -> Vec<String> {
    let mut undefined_vars = Vec::new();
    
    // 简单的正则表达式来提取 {{ variable_name }} 格式的变量
    // 这里使用简单的字符串解析，不需要正则表达式库
    let mut i = 0;
    
    while i < template.len() {
        if template[i..].starts_with("{{") {
            // 找到模板开始
            i += 2;
            
            // 跳过空格
            while i < template.len() && template.chars().nth(i).unwrap_or(' ').is_whitespace() {
                i += 1;
            }
            
            // Check if this is a string literal (starts with ' or ")
            if i < template.len() {
                let ch = template.chars().nth(i).unwrap_or(' ');
                if ch == '\'' || ch == '"' {
                    // This is a string literal, skip checking for undefined variables
                    debug!("Skipping variable check for string literal in template: {}", template);
                    break;
                }
            }
            
            // 提取变量名（直到空格、| 或 }}）
            let var_start = i;
            while i < template.len() {
                let ch = template.chars().nth(i).unwrap_or(' ');
                if ch.is_whitespace() || ch == '|' || template[i..].starts_with("}}") {
                    break;
                }
                i += 1;
            }
            
            if i > var_start {
                let var_name = template[var_start..i].to_string();
                
                // Skip common ansible built-in variables that might not be in context
                if ["inventory_hostname", "ansible_hostname", "ansible_host", "ansible_port",
                    "ansible_ssh_user", "ansible_user", "group_names", "groups"].contains(&var_name.as_str()) {
                    debug!("Skipping check for built-in Ansible variable: {}", var_name);
                } else {
                    // 检查变量是否在上下文中定义
                    if !context.contains_key(&var_name) {
                        undefined_vars.push(var_name);
                    }
                }
            }
        } else {
            i += 1;
        }
    }
    
    undefined_vars
}

/// Helper function to extract undefined variables from a template
fn extract_undefined_variables(template: &str, context: &TeraContext) -> Vec<String> {
    check_template_variables(template, context)
}

/// Helper function to convert a variable map to a Tera Context.
/// This should ideally happen once before processing a task or loop.
pub fn create_tera_context(vars: &HashMap<String, Value>) -> TeraContext {
    let mut context = TeraContext::new();
    for (key, value) in vars {
        // Convert serde_yaml::Value back to serde_json::Value for Tera context
        match serde_json::to_value(value) {
            Ok(json_val) => context.insert(key, &json_val),
            Err(e) => {
                warn!("Could not convert variable '{}' for Tera context: {}", key, e);
                // Optionally insert a Null or skip, depending on desired behavior
            }
        }
    }
    context
}

/// Evaluate a condition expression using Tera.
///
/// # Arguments
/// * `condition` - The condition string (e.g., "item.enabled == true").
/// * `tera` - A reference to the Tera instance with registered filters.
/// * `context` - The Tera context containing variables for evaluation.
pub fn evaluate_condition(condition: &str, tera: &mut Tera, context: &TeraContext) -> Result<bool> {
    debug!("Evaluating condition with Tera: {}", condition);

    // Render the condition expression directly
    let template = format!("{{{{ {} }}}}", condition);
    match render_value(&template, tera, context, false) {
        Ok(result_value) => {
            // Evaluate truthiness of the resulting Value
            Ok(evaluate_truthiness(&result_value))
        },
        Err(e) => {
            warn!("Error evaluating condition '{}' with Tera: {}", condition, e);
            Err(anyhow!("Tera condition evaluation error for '{}': {}", condition, e))
        }
    }
}

// Helper function to evaluate the truthiness of a serde_yaml::Value
fn evaluate_truthiness(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Number(n) => {
            if let Some(i) = n.as_i64() { i != 0 }
            else if let Some(u) = n.as_u64() { u != 0 }
            else if let Some(f) = n.as_f64() { f != 0.0 }
            else { false } // Should not happen with standard numbers
        },
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
    let mut result = input.to_string();
    
    // 处理 password_hash filter
    if result.contains("password_hash(") {
        // 匹配 password_hash('value') 并转换为 password_hash(hash_type='value')
        result = result.replace("password_hash('sha512')", "password_hash(hash_type='sha512')");
        result = result.replace("password_hash('sha256')", "password_hash(hash_type='sha256')");
        result = result.replace("password_hash('md5')", "password_hash(hash_type='md5')");
        result = result.replace("password_hash('bcrypt')", "password_hash(hash_type='bcrypt')");
        
        // 处理动态值的情况
        result = result.replace("password_hash(\"sha512\")", "password_hash(hash_type=\"sha512\")");
        result = result.replace("password_hash(\"sha256\")", "password_hash(hash_type=\"sha256\")");
        result = result.replace("password_hash(\"md5\")", "password_hash(hash_type=\"md5\")");
        result = result.replace("password_hash(\"bcrypt\")", "password_hash(hash_type=\"bcrypt\")");
    }
    
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::Mapping;

    // Helper to create Tera instance for tests
    fn create_test_tera() -> Tera {
        let tera = Tera::default();
        tera
    }

    // Helper to create Tera context from HashMap<String, Value>
    fn create_test_context_from_map(vars: &HashMap<String, Value>) -> TeraContext {
        create_tera_context(vars)
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

         let result = render_value("{% if use_prod %}production{% else %}staging{% endif %}", &mut tera, &context, false).unwrap();
         assert_eq!(result, Value::String("production".to_string()));

         let mut vars2 = HashMap::new();
         vars2.insert("use_prod".to_string(), Value::Bool(false));
         let context2 = create_test_context_from_map(&vars2);
         let result2 = render_value("{% if use_prod %}production{% else %}staging{% endif %}", &mut tera, &context2, false).unwrap();
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
        vars.insert("my_list".to_string(), Value::Sequence(vec![Value::String("a".into()), Value::String("b".into())]));
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
         assert!(evaluate_condition("item.enabled and item.level > 1", &mut tera, &context).unwrap());
         assert!(!evaluate_condition("item.enabled and item.level < 1", &mut tera, &context).unwrap());
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
         let result = render_value("{{ root_password | password_hash('sha512') }}", &mut tera, &context, false);
         
         // 应该返回错误
         assert!(result.is_err());
         
         let error = result.unwrap_err();
         println!("Error: {}", error);
         
         // 错误应该指示变量未定义
         let error_string = format!("{}", error);
         assert!(error_string.contains("Variable") || error_string.contains("not found") || error_string.contains("root_password"));
     }

     #[test]
     fn test_render_value_defined_variable_with_filter() {
         let mut tera = create_test_tera();
         // 注册 password_hash filter
         register_ansible_filters(&mut tera);
         
         // 创建包含定义变量的上下文
         let mut vars = HashMap::new();
         vars.insert("test_password".to_string(), Value::String("mypassword123".to_string()));
         let context = create_test_context_from_map(&vars);
         
         // 测试已定义变量与 filter 的组合
         let result = render_value("{{ test_password | password_hash('sha512') }}", &mut tera, &context, false);
         
         // 打印结果以便调试
         match &result {
             Ok(rendered) => {
                 println!("Success: {:?}", rendered);
             },
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
         vars.insert("test_password".to_string(), Value::String("mypassword123".to_string()));
         let context = create_test_context_from_map(&vars);
         
         // 测试简单的变量渲染（不使用 filter）
         let result = render_value("{{ test_password }}", &mut tera, &context, false);
         println!("Simple variable result: {:?}", result);
         assert!(result.is_ok());
         
         // 测试 Tera 的 render_str 直接调用
         let direct_result = tera.render_str("{{ test_password }}", &context);
         println!("Direct Tera result: {:?}", direct_result);
         assert!(direct_result.is_ok());
         
         // 注册 filter 并测试
         register_ansible_filters(&mut tera);
         
         // 测试 filter 是否注册成功
         let filter_result = tera.render_str("{{ test_password | password_hash('sha512') }}", &context);
         println!("Filter result with quotes: {:?}", filter_result);
         
         // 尝试不同的语法
         let filter_result2 = tera.render_str("{{ test_password | password_hash(hash_type='sha512') }}", &context);
         println!("Filter result with named param: {:?}", filter_result2);
         
         // 尝试没有引号的语法
         let filter_result3 = tera.render_str("{{ test_password | password_hash(sha512) }}", &context);
         println!("Filter result without quotes: {:?}", filter_result3);
     }

} 