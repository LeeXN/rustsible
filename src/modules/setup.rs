use anyhow::{Context, Result};
use serde_yaml::Value;
use std::collections::HashMap;

use crate::inventory::Host;
use crate::modules::python::execute_json_mapping;
use crate::modules::ModuleResult;
use crate::ssh::connection::{Connection, SshConnection};

const SETUP_SCRIPT: &str = r#"import json
import datetime
import time
import platform
import sys

if sys.platform != "linux":
    raise RuntimeError("unsupported managed platform: expected Linux")

machine = platform.machine().lower()
architecture = {
    "amd64": "x86_64",
    "x64": "x86_64",
    "arm64": "aarch64",
}.get(machine, machine)

now = datetime.datetime.now().astimezone()
print(json.dumps({
    "ansible_facts": {
        "architecture": architecture,
        "system": "Linux",
        "date_time": {
            "date": now.strftime("%Y-%m-%d"), "time": now.strftime("%H:%M:%S"),
            "year": now.strftime("%Y"), "month": now.strftime("%m"),
            "day": now.strftime("%d"), "hour": now.strftime("%H"),
            "minute": now.strftime("%M"), "second": now.strftime("%S"),
            "weekday": now.strftime("%A"), "weekday_number": now.strftime("%w"),
            "epoch": str(int(time.time())),
            "iso8601": now.astimezone(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        },
    }
}))
"#;

pub fn execute(
    connection: &dyn SshConnection,
    use_become: bool,
    become_user: &str,
) -> Result<ModuleResult> {
    let output = execute_json_mapping(connection, SETUP_SCRIPT, &[], use_become, become_user)?;
    let facts = output
        .get(Value::String("ansible_facts".to_string()))
        .cloned()
        .context("Fact gathering did not return ansible_facts")?;
    let mut values = HashMap::new();
    values.insert("ansible_facts".to_string(), facts);

    Ok(ModuleResult {
        changed: false,
        msg: "Facts gathered".to_string(),
        values,
        ..ModuleResult::default()
    })
}

pub(crate) fn validate_args(args: &Value) -> Result<()> {
    if !args.as_mapping().is_some_and(|mapping| mapping.is_empty()) {
        anyhow::bail!("Setup does not support arguments in this compatibility scope");
    }
    Ok(())
}

pub fn execute_adhoc(
    host: &Host,
    args: &Value,
    use_become: bool,
    become_user: &str,
    _check_mode: bool,
) -> Result<ModuleResult> {
    validate_args(args)?;
    let connection = Connection::connect(host)?;
    execute(connection.as_connection(), use_become, become_user)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::Host;
    use crate::ssh::connection::{LocalConnection, MockSshConnection};

    #[test]
    fn gathers_linux_architecture_with_ansible_names() {
        let mut host = Host::new("localhost");
        host.set_variable("ansible_connection", "local");
        let connection = LocalConnection::new(&host).unwrap();

        let result = execute(&connection, false, "root").unwrap();
        let facts = result.values["ansible_facts"].as_mapping().unwrap();
        assert_eq!(
            facts.get(Value::String("system".to_string())),
            Some(&Value::String("Linux".to_string()))
        );
        assert!(matches!(
            facts.get(Value::String("architecture".to_string())),
            Some(Value::String(architecture)) if !architecture.is_empty()
        ));
    }

    #[test]
    fn missing_python_is_a_diagnostic_error() {
        let mut connection = MockSshConnection::new();
        connection
            .expect_execute_command_with_input()
            .once()
            .returning(|_, _| Ok((127, String::new(), "python3: not found".to_string())));

        let error = execute(&connection, false, "root").unwrap_err();
        assert!(error.to_string().contains("Python 3 is required"));
    }

    #[test]
    fn ad_hoc_setup_rejects_unsupported_arguments_before_connecting() {
        let args = serde_yaml::from_str("filter: ansible_architecture").unwrap();
        let error = execute_adhoc(
            &Host::new("unreachable.invalid"),
            &args,
            false,
            "root",
            false,
        )
        .unwrap_err();
        assert!(error.to_string().contains("does not support arguments"));
    }
}
