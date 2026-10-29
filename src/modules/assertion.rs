use super::{registry::Builtin, ModuleResult};
use anyhow::{Context, Result};
use serde_yaml::Value;
use std::collections::HashMap;

pub(crate) fn execute(
    module: Builtin,
    args: &Value,
    variables: &HashMap<String, Value>,
) -> Result<ModuleResult> {
    module.validate_parameters(args)?;
    let message = |name: &str, fallback: &str| {
        args.get(name)
            .and_then(Value::as_str)
            .unwrap_or(fallback)
            .to_owned()
    };
    if module == Builtin::Fail {
        return Ok(ModuleResult {
            failed: true,
            msg: message("msg", "Failed as requested from task"),
            ..Default::default()
        });
    }
    let that = args.get("that").context("assert requires that")?;
    let conditions = if let Value::Sequence(conditions) = that {
        conditions.as_slice()
    } else {
        std::slice::from_ref(that)
    };
    let context = crate::playbook::templar::create_context(variables)?;
    let mut environment = crate::playbook::templar::environment();
    for condition in conditions {
        if !crate::playbook::templar::evaluate_conditions(condition, &mut environment, &context)? {
            return Ok(ModuleResult {
                failed: true,
                msg: message(
                    "fail_msg",
                    args.get("msg")
                        .and_then(Value::as_str)
                        .unwrap_or("Assertion failed"),
                ),
                values: HashMap::from([
                    ("assertion".into(), condition.clone()),
                    ("evaluated_to".into(), Value::Bool(false)),
                ]),
                ..Default::default()
            });
        }
    }
    Ok(ModuleResult {
        msg: message("success_msg", "All assertions passed"),
        values: HashMap::from([(
            "_ansible_verbose_always".into(),
            Value::Bool(args.get("quiet").and_then(Value::as_bool) != Some(true)),
        )]),
        ..Default::default()
    })
}
