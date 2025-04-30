use anyhow::{Result, anyhow};
use log::{debug, warn};
use std::collections::HashMap;
use serde_yaml::Value;
use tera::{Tera, Context as TeraContext};

const MAX_TEMPLATE_RECURSION: usize = 10;

/// Render a string value using Tera templating, handling recursion.
///
/// # Arguments
/// * `input` - The string potentially containing Tera expressions.
/// * `tera` - A reference to the Tera instance.
/// * `context` - The Tera context containing variables.
pub fn render_value(input: &str, tera: &mut Tera, context: &TeraContext) -> Result<Value> {
    debug!("Rendering value with Tera (initial): {}", input);
    let mut current_str = input.to_string();
    let mut depth = 0;

    // Loop for recursive rendering
    while depth < MAX_TEMPLATE_RECURSION {
        // Removed check for specific markers: rely on fixed-point check below
        /* if !current_str.contains("{{") || !current_str.contains("}}") {
            debug!("No more template markers found at depth {}", depth);
            break;
        }*/

        depth += 1;
        debug!("Rendering value (depth {}): {}", depth, current_str);
        let last_str = current_str.clone(); // Store previous value to detect no change

        match tera.render_str(&current_str, context) {
            Ok(rendered) => {
                 if rendered == last_str { // No change occurred, break loop
                     debug!("Template rendering reached fixed point at depth {}", depth);
                     break;
                 }
                current_str = rendered;
            },
            Err(e) => {
                // If rendering fails at any depth, return the error
                // Check if the error is because a variable is undefined *in this specific pass*
                // If the original input didn't contain this variable, it might be okay?
                // For now, let's be strict: any render error is an error.
                warn!("Error rendering Tera template '{}' at depth {}: {}", input, depth, e);
                return Err(anyhow!("Tera rendering error for '{}' at depth {}: {}", input, depth, e));
            }
        }
    }

    if depth >= MAX_TEMPLATE_RECURSION {
        warn!(
            "Template rendering exceeded maximum recursion depth ({}) for input: {}. Final string: {}",
            MAX_TEMPLATE_RECURSION, input, current_str
        );
        // Return the last successfully rendered string, even if recursion limit hit
    }

    debug!("Final rendered string after recursion: {}", current_str);

    // --- Logic to parse the final string (JSON/YAML or fallback to string) ---
    // Try parsing as JSON first (more standard for filter outputs or rendered results)
    match serde_json::from_str::<serde_json::Value>(&current_str) {
        Ok(json_value) => {
            // Convert serde_json::Value to serde_yaml::Value
            match serde_yaml::to_value(json_value) {
                Ok(yaml_value) => Ok(yaml_value),
                Err(e) => {
                    warn!("Failed to convert final rendered JSON value '{}' to YAML: {}. Using string.", current_str, e);
                    Ok(Value::String(current_str))
                }
            }
        },
        Err(_) => {
            // If JSON fails, try YAML (might handle bare strings like 'true'/'false' differently)
            match serde_yaml::from_str(&current_str) {
                Ok(yaml_value) => Ok(yaml_value),
                Err(_) => {
                     // If both JSON and YAML parsing fail, return as plain string
                    Ok(Value::String(current_str))
                }
            }
        }
    }
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
    match render_value(&template, tera, context) {
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

        let result = render_value("Hello {{ name }}!", &mut tera, &context).unwrap();
        assert_eq!(result, Value::String("Hello World!".to_string()));
    }

     #[test]
     fn test_render_value_complex_expression() {
         let mut tera = create_test_tera();
         let mut vars = HashMap::new();
         vars.insert("a".to_string(), Value::Number(5.into()));
         vars.insert("b".to_string(), Value::Number(3.into()));
         let context = create_test_context_from_map(&vars);

         let result = render_value("{{ a + b * 2 }}", &mut tera, &context).unwrap();
         // Tera should evaluate this to 11
         assert_eq!(result, Value::Number(serde_yaml::Number::from(11)));
     }

     #[test]
     fn test_render_value_conditional_expression() {
         let mut tera = create_test_tera();
         let mut vars = HashMap::new();
         vars.insert("use_prod".to_string(), Value::Bool(true));
         let context = create_test_context_from_map(&vars);

         let result = render_value("{% if use_prod %}production{% else %}staging{% endif %}", &mut tera, &context).unwrap();
         assert_eq!(result, Value::String("production".to_string()));

         let mut vars2 = HashMap::new();
         vars2.insert("use_prod".to_string(), Value::Bool(false));
         let context2 = create_test_context_from_map(&vars2);
         let result2 = render_value("{% if use_prod %}production{% else %}staging{% endif %}", &mut tera, &context2).unwrap();
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

} 