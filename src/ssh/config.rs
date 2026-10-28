use anyhow::{bail, Result};
use serde_yaml::Value;
use std::borrow::Cow;
use std::collections::HashMap;

use crate::inventory::Host;

/// An immutable snapshot of the final variable context. The transport and its
/// credentials use these exact values; they never evaluate template text.
#[derive(Clone)]
pub struct ConnectionConfig {
    pub(crate) name: String,
    pub(crate) hostname: String,
    pub(crate) port: u16,
    pub(crate) use_become: bool,
    pub(crate) become_user: String,
    variables: HashMap<String, Value>,
}

impl ConnectionConfig {
    pub fn hostname(&self) -> &str {
        &self.hostname
    }
    pub fn port(&self) -> u16 {
        self.port
    }
    pub fn from_host(host: &Host) -> Result<Self> {
        let definitions = host.all_variables();
        let data = HashMap::from([(
            "inventory_hostname".into(),
            Value::String(host.name.clone()),
        )]);
        let resolved = crate::playbook::templar::resolve_definitions(&definitions, &data)?;
        Self::require_resolved(&definitions, &resolved)?;
        Self::from_context(&host.name, resolved)
    }

    pub(crate) fn require_resolved(
        definitions: &HashMap<String, Value>,
        values: &HashMap<String, Value>,
    ) -> Result<()> {
        for original_name in definitions.keys() {
            let name = crate::inventory::host::canonical_variable_name(original_name);
            if is_connection_variable(name) && !values.contains_key(name) {
                bail!("Cannot resolve connection variable '{name}'");
            }
        }
        Ok(())
    }

    pub fn from_context(name: &str, variables: HashMap<String, Value>) -> Result<Self> {
        let mut normalized = HashMap::new();
        crate::inventory::host::merge_variable_layer(&mut normalized, &variables);
        let variables = normalized;
        for (name, value) in &variables {
            if crate::inventory::host::canonical_variable_name(name) != name {
                continue;
            }
            crate::inventory::parser::validate_resolved_yaml_connection_variable(name, value)?;
            if let Some(text) = scalar_text(value) {
                crate::inventory::parser::validate_resolved_connection_variable(name, &text, 0)?;
            }
        }
        let host = Host {
            name: name.to_owned(),
            variables,
            inherited_variables: HashMap::new(),
        };
        let hostname = host.hostname()?.to_owned();
        let port = host.port()?;
        for (key, supported) in [
            ("ansible_become_method", "sudo"),
            ("ansible_become_exe", "sudo"),
        ] {
            if host
                .get_variable(key)
                .is_some_and(|value| value.as_str() != Some(supported))
            {
                bail!("Only {key}={supported} is supported");
            }
        }
        if host
            .get_variable("ansible_become_flags")
            .is_some_and(|value| value.as_str() != Some(""))
        {
            bail!("Custom ansible_become_flags are not supported");
        }
        let use_become = host.get_become().unwrap_or(false);
        let become_user = host.get_become_user().unwrap_or("root").to_owned();
        if become_user.trim().is_empty() || become_user.chars().any(char::is_control) {
            bail!("ansible_become_user must be non-empty without control characters");
        }
        Ok(Self {
            name: host.name,
            hostname,
            port,
            use_become,
            become_user,
            variables: host.variables,
        })
    }

    pub fn get_variable(&self, key: &str) -> Option<Cow<'_, str>> {
        self.variables.get(key).and_then(scalar_text)
    }
    pub fn get_ssh_user(&self) -> Option<&str> {
        self.variables.get("ansible_user").and_then(Value::as_str)
    }
    pub fn get_ssh_password(&self) -> Option<&str> {
        self.variables
            .get("ansible_password")
            .and_then(Value::as_str)
    }
    pub fn get_ssh_private_key(&self) -> Option<&str> {
        self.variables
            .get("ansible_ssh_private_key_file")
            .and_then(Value::as_str)
    }
    pub fn get_ssh_sudo_password(&self) -> Option<&str> {
        self.variables
            .get("ansible_become_password")
            .and_then(Value::as_str)
    }
}

fn scalar_text(value: &Value) -> Option<Cow<'_, str>> {
    match value {
        Value::String(value) => Some(Cow::Borrowed(value)),
        Value::Bool(value) => Some(Cow::Owned(value.to_string())),
        Value::Number(value) => Some(Cow::Owned(value.to_string())),
        _ => None,
    }
}

fn is_connection_variable(name: &str) -> bool {
    matches!(
        name,
        "ansible_host"
            | "ansible_ssh_host"
            | "ansible_port"
            | "ansible_ssh_port"
            | "ansible_user"
            | "ansible_ssh_user"
            | "ansible_password"
            | "ansible_ssh_pass"
            | "ansible_become"
            | "ansible_sudo"
            | "ansible_become_user"
            | "ansible_sudo_user"
            | "ansible_become_password"
            | "ansible_become_pass"
            | "ansible_sudo_pass"
            | "ansible_ssh_sudo_pass"
            | "ansible_ssh_private_key_file"
            | "ansible_connection"
            | "ansible_connection_type"
            | "ansible_ssh_timeout"
            | "ansible_timeout"
            | "rustsible_command_timeout"
            | "ansible_command_timeout"
            | "rustsible_host_key_checking"
            | "ansible_host_key_checking"
            | "ansible_ssh_host_key_checking"
            | "rustsible_known_hosts_file"
            | "ansible_ssh_known_hosts_file"
            | "ansible_become_method"
            | "ansible_become_exe"
            | "ansible_become_flags"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_values_shadow_unused_alias_definitions() {
        let mut host = Host::new("node");
        host.set_variable("ansible_host", "127.0.0.1");
        host.set_variable("ansible_ssh_host", "{{ missing_lower_priority_value }}");
        assert_eq!(
            ConnectionConfig::from_host(&host).unwrap().hostname(),
            "127.0.0.1"
        );
        host.set_typed_variable("ansible_ssh_host", Value::Bool(false));
        assert_eq!(
            ConnectionConfig::from_host(&host).unwrap().hostname(),
            "127.0.0.1"
        );
    }
    #[test]
    fn resolved_context_is_data_and_does_not_execute_template_symbols() {
        let values = HashMap::from([
            (
                "ansible_host".into(),
                Value::String("{{ literal_data }}".into()),
            ),
            ("ansible_become".into(), Value::String(" true ".into())),
        ]);
        let config = ConnectionConfig::from_context("node", values).unwrap();
        assert_eq!(config.hostname(), "{{ literal_data }}");
        assert!(config.use_become);
    }
}
