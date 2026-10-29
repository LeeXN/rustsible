use serde_yaml::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok,
    Failed,
    Skipped,
    Unreachable,
    Uncertain,
}
impl Outcome {
    pub fn from_flags(failed: bool, skipped: bool, unreachable: bool, uncertain: bool) -> Self {
        if uncertain {
            Self::Uncertain
        } else if unreachable {
            Self::Unreachable
        } else if failed {
            Self::Failed
        } else if skipped {
            Self::Skipped
        } else {
            Self::Ok
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::Unreachable => "unreachable",
            Self::Uncertain => "uncertain",
        }
    }
}

/// Result structure for unified handling of module returns
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleResult {
    pub stdout: String,
    pub stderr: String,
    pub rc: Option<i32>,
    pub changed: bool,
    pub failed: bool,
    pub skipped: bool,
    pub unreachable: bool,
    pub uncertain: bool,
    pub msg: String,
    /// Module-specific return values exposed to `register`.
    pub values: HashMap<String, Value>,
}

impl ModuleResult {
    pub fn outcome(&self) -> Outcome {
        Outcome::from_flags(
            self.failed || self.rc.is_some_and(|rc| rc != 0),
            self.skipped,
            self.unreachable,
            self.uncertain,
        )
    }
    pub fn as_value(&self) -> Value {
        let mut values = self.values.clone();
        insert_output_values(
            &mut values,
            self.stdout.clone(),
            self.stderr.clone(),
            self.rc,
        );
        Value::Mapping(result_fields(
            self.changed,
            self.outcome(),
            &self.msg,
            &values,
        ))
    }
}

/// Extra module fields cannot override the common execution outcome when
/// flattened into an Ansible-style registered object.
pub fn result_fields(
    changed: bool,
    outcome: Outcome,
    message: &str,
    values: &std::collections::HashMap<String, serde_yaml::Value>,
) -> serde_yaml::Mapping {
    use serde_yaml::Value;
    let mut value = values
        .iter()
        .map(|(key, value)| (Value::String(key.clone()), value.clone()))
        .collect::<serde_yaml::Mapping>();
    for (key, flag) in [
        ("changed", changed),
        (
            "failed",
            matches!(
                outcome,
                Outcome::Failed | Outcome::Unreachable | Outcome::Uncertain
            ),
        ),
        ("skipped", outcome == Outcome::Skipped),
        ("unreachable", outcome == Outcome::Unreachable),
        ("uncertain", outcome == Outcome::Uncertain),
    ] {
        value.insert(Value::String(key.into()), Value::Bool(flag));
    }
    value.insert(
        Value::String("outcome".into()),
        Value::String(outcome.as_str().into()),
    );
    value.insert(Value::String("msg".into()), Value::String(message.into()));
    value
}

pub fn insert_output_values(
    values: &mut HashMap<String, Value>,
    stdout: String,
    stderr: String,
    rc: Option<i32>,
) {
    let stdout_lines = stdout
        .lines()
        .map(|line| Value::String(line.to_string()))
        .collect();
    let stderr_lines = stderr
        .lines()
        .map(|line| Value::String(line.to_string()))
        .collect();
    values.insert("stdout".to_string(), Value::String(stdout));
    values.insert("stderr".to_string(), Value::String(stderr));
    values.insert("stdout_lines".to_string(), Value::Sequence(stdout_lines));
    values.insert("stderr_lines".to_string(), Value::Sequence(stderr_lines));
    values.insert(
        "rc".to_string(),
        rc.map_or(Value::Null, |code| Value::Number(code.into())),
    );
}
