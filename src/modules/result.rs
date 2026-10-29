use super::ModuleResult;
use crate::ssh::connection::TransportFailure;
use serde_yaml::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// A connection failure happens before module dispatch; it must not be
/// confused with losing the response of a potentially modifying task.
#[derive(Debug)]
pub struct ExecutionFailure {
    outcome: Outcome,
    message: String,
}
impl ExecutionFailure {
    pub fn unreachable(error: anyhow::Error) -> Self {
        Self {
            outcome: Outcome::Unreachable,
            message: format!("Failed to connect to host: {error:#}"),
        }
    }
    pub fn into_error(self) -> anyhow::Error {
        anyhow::Error::new(self)
    }
}
impl std::fmt::Display for ExecutionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for ExecutionFailure {}

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
    pub(crate) fn from_error(error: anyhow::Error, may_modify: bool) -> Self {
        let outcome = if let Some(failure) = error.downcast_ref::<ExecutionFailure>() {
            failure.outcome
        } else if let Some(failure) = error.downcast_ref::<TransportFailure>() {
            if may_modify && failure.started {
                Outcome::Uncertain
            } else if failure.remote {
                Outcome::Unreachable
            } else {
                Outcome::Failed
            }
        } else {
            Outcome::Failed
        };
        let mut messages = Vec::new();
        for cause in error.chain() {
            let message = cause.to_string();
            if messages.last() != Some(&message) {
                messages.push(message);
            }
        }
        let output = error.downcast_ref::<CommandOutput>();
        Self {
            changed: known_changed(),
            rc: output.map(|output| output.rc),
            stdout: output
                .map(|output| output.stdout.clone())
                .unwrap_or_default(),
            stderr: output
                .map(|output| output.stderr.clone())
                .unwrap_or_default(),
            failed: true,
            uncertain: outcome == Outcome::Uncertain,
            unreachable: outcome == Outcome::Unreachable,
            msg: messages.join(": "),
            ..Self::default()
        }
    }
}

/// Extra module fields cannot override the common execution outcome when
/// flattened into an Ansible-style registered object.
pub(crate) fn result_fields(
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extra_values_cannot_forge_a_success_or_hide_uncertainty() {
        let values = std::collections::HashMap::from([
            ("failed".into(), serde_yaml::Value::Bool(false)),
            ("uncertain".into(), serde_yaml::Value::Bool(false)),
        ]);
        let fields = result_fields(true, Outcome::Uncertain, "response lost", &values);
        assert_eq!(fields["failed"], serde_yaml::Value::Bool(true));
        assert_eq!(fields["uncertain"], serde_yaml::Value::Bool(true));
        assert_eq!(
            fields["outcome"],
            serde_yaml::Value::String("uncertain".into())
        );
    }
}

thread_local! { static CHANGE_STATE: std::cell::Cell<Option<(bool, bool)>> = const { std::cell::Cell::new(None) }; }

/// Invocation-local accounting avoids mixing hosts on concurrent/reused
/// threads. Only explicitly mutating, successful operations record a change.
pub(crate) struct ChangeScope(Option<(bool, bool)>);
impl ChangeScope {
    pub fn enter(check: bool) -> Self {
        let prior = CHANGE_STATE.get();
        let check = check || prior.is_some_and(|(check, _)| check);
        CHANGE_STATE.set(Some((check, false)));
        Self(prior)
    }
}
impl Drop for ChangeScope {
    fn drop(&mut self) {
        let changed = known_changed();
        CHANGE_STATE.set(self.0.map(|(check, prior)| (check, prior || changed)));
    }
}
pub(crate) fn is_check_active() -> bool {
    CHANGE_STATE.get().is_some_and(|(check, _)| check)
}
pub(crate) fn known_changed() -> bool {
    CHANGE_STATE.get().is_some_and(|(_, changed)| changed)
}
pub(crate) fn ensure_mutation_allowed() -> anyhow::Result<()> {
    if CHANGE_STATE.get().is_some_and(|(check, _)| check) {
        anyhow::bail!("Check mode forbids target mutations");
    }
    Ok(())
}
pub(crate) fn record_change() {
    CHANGE_STATE.set(CHANGE_STATE.get().map(|(check, _)| (check, true)));
}

#[derive(Debug)]
struct CommandOutput {
    rc: i32,
    stdout: String,
    stderr: String,
    message: String,
}
impl std::fmt::Display for CommandOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for CommandOutput {}
pub(crate) fn command_error(
    error: anyhow::Error,
    rc: i32,
    stdout: String,
    stderr: String,
) -> anyhow::Error {
    let message = error.to_string();
    error.context(CommandOutput {
        rc,
        stdout,
        stderr,
        message,
    })
}

pub(crate) fn insert_output_values(
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
mod accounting_tests {
    use super::*;
    #[test]
    fn failures_retain_known_changes_and_command_outputs_without_leaking_to_next_invocation() {
        {
            let _scope = ChangeScope::enter(false);
            record_change();
            let error = command_error(
                anyhow::anyhow!("second operation failed"),
                42,
                "available output".into(),
                "available error".into(),
            );
            let result = ModuleResult::from_error(error, true);
            assert!(result.changed);
            assert_eq!(result.rc, Some(42));
            assert_eq!(result.stdout, "available output");
            assert_eq!(result.stderr, "available error");
            assert_eq!(result.outcome(), Outcome::Failed);
        }
        let _scope = ChangeScope::enter(false);
        assert!(!known_changed());
    }
    #[test]
    fn nested_invocations_cannot_leave_global_check_mode() {
        let _outer = ChangeScope::enter(true);
        let _inner = ChangeScope::enter(false);
        assert!(ensure_mutation_allowed().is_err());
        let invocation = crate::modules::registry::Invocation::prepare(
            "shell",
            &Value::String("exit 99".into()),
            false,
        )
        .unwrap();
        assert_eq!(
            invocation.execute(None, &HashMap::new()).outcome(),
            Outcome::Skipped
        );
    }
}
