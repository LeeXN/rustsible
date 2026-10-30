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
    /// A private async result keeps its display policy when queried later.
    #[serde(default)]
    pub no_log: bool,
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
    /// Providers share the same display bounds as builtin diff producers. Keep
    /// binary/large strings as digests and cap aggregate structure size too.
    pub fn limit_diff(&mut self) {
        fn summary(bytes: &[u8]) -> Value {
            let mut input = bytes;
            let digest = crate::sha256(&mut input).expect("Reading an in-memory diff cannot fail");
            serde_yaml::to_value(
                serde_json::json!({"size":bytes.len(),"sha256":digest,"content_omitted":true}),
            )
            .expect("Diff summary is serializable")
        }
        fn bound(value: &mut Value) {
            match value {
                Value::String(text) if text.len() > 64 * 1024 || text.contains('\0') => {
                    *value = summary(text.as_bytes())
                }
                Value::Sequence(values) => values.iter_mut().for_each(bound),
                Value::Mapping(values) => values.values_mut().for_each(bound),
                _ => {}
            }
        }
        if let Some(diff) = self.values.get_mut("diff") {
            bound(diff);
            if let Ok(bytes) = serde_json::to_vec(diff) {
                if bytes.len() > 128 * 1024 {
                    *diff = summary(&bytes);
                }
            }
        }
    }

    pub fn displayed(&self) -> Self {
        if !self.no_log {
            return self.clone();
        }
        Self {
            no_log: true,
            changed: self.changed,
            failed: self.failed,
            skipped: self.skipped,
            unreachable: self.unreachable,
            uncertain: self.uncertain,
            rc: self.rc,
            msg: "output censored because no_log is enabled".into(),
            ..Default::default()
        }
    }
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

#[cfg(test)]
mod diff_tests {
    use super::*;
    #[test]
    fn external_diff_cannot_exceed_builtin_text_binary_or_aggregate_display_bounds() {
        let mut result = ModuleResult::default();
        result.values.insert("diff".into(),serde_yaml::to_value(serde_json::json!({"before":"é".repeat(40000),"after":"binary\0data","marker":"retained"})).unwrap());
        result.limit_diff();
        let diff = &result.values["diff"];
        assert_eq!(diff["before"]["size"].as_u64(), Some(80000));
        assert_eq!(diff["before"]["sha256"].as_str().unwrap().len(), 64);
        assert_eq!(diff["after"]["content_omitted"].as_bool(), Some(true));
        assert_eq!(diff["marker"].as_str(), Some("retained"));
        result.values.insert(
            "diff".into(),
            serde_yaml::to_value(vec!["a".repeat(1000); 200]).unwrap(),
        );
        result.limit_diff();
        assert_eq!(
            result.values["diff"]["content_omitted"].as_bool(),
            Some(true)
        );
        assert!(serde_json::to_vec(&result.values["diff"]).unwrap().len() < 1024);
    }
}
