use super::ModuleResult;
use crate::ssh::connection::TransportFailure;
#[cfg(test)]
use serde_yaml::Value;
#[cfg(test)]
use std::collections::HashMap;

pub use rustsible_protocol::Outcome;
pub(crate) use rustsible_protocol::{insert_output_values, result_fields};

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

pub(crate) fn from_error(error: anyhow::Error, may_modify: bool) -> ModuleResult {
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
    ModuleResult {
        changed: known_changed(),
        failed: true,
        uncertain: outcome == Outcome::Uncertain,
        unreachable: outcome == Outcome::Unreachable,
        msg: messages.join(": "),
        ..ModuleResult::default()
    }
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

pub(crate) use rustsible_protocol::change::{is_check_active, known_changed, ChangeScope};

#[cfg(test)]
mod accounting_tests {
    use super::*;
    use rustsible_protocol::change::{ensure_mutation_allowed, record_change};
    #[test]
    fn failures_retain_known_changes_and_command_outputs_without_leaking_to_next_invocation() {
        {
            let _scope = ChangeScope::enter(false);
            record_change();
            let error = anyhow::anyhow!("second operation failed");
            let result = from_error(error, true);
            assert!(result.changed);
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
            invocation
                .execute(None, &HashMap::new(), &crate::worker::Runtime::default())
                .outcome(),
            Outcome::Skipped
        );
    }
}
