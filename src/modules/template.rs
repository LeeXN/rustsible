use crate::playbook::templar;
use anyhow::{Context, Result};
use minijinja::value::Serde;
use serde_yaml::Value;
use std::{collections::HashMap, fs, path::Path};

/// Render on the controller; the target only receives bytes and installation
/// metadata, so it never evaluates controller variables or templates.
pub(crate) fn render(template_args: &Value, data: &HashMap<String, Value>) -> Result<String> {
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

    let mut definitions = HashMap::new();
    if let Some(vars) = template_args.get("vars") {
        let vars = vars
            .as_mapping()
            .context("Template 'vars' parameter must be a mapping")?;
        for (name, value) in vars {
            let name = name
                .as_str()
                .context("Template variable names must be strings")?;
            definitions.insert(name.to_string(), value.clone());
        }
    }
    let vars = templar::resolve_definitions(&definitions, data)?;
    let context = templar::create_context(&vars)?;
    let source_name = template_args
        .get("src")
        .and_then(Value::as_str)
        .unwrap_or("<inline template>");
    templar::environment()
        .render_named_str(source_name, &template_string, Serde(&context))
        .map_err(Into::into)
}
