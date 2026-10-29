use anyhow::{bail, Context, Result};
use log::debug;
use serde_yaml::Value;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

/// Inventory data has one typed table per layer. Connection fields are derived
/// only when constructing the execution configuration, never cached on Host.
#[derive(Debug, Clone)]
pub struct Host {
    pub name: String,
    pub variables: HashMap<String, Value>,
    pub inherited_variables: HashMap<String, Value>,
    /// One shared configuration baseline stays below play/task keywords and
    /// explicit inventory variables; it is never another mutable host copy.
    pub(crate) defaults: std::sync::Arc<HashMap<String, Value>>,
}

const VARIABLE_ALIASES: &[(&str, &str)] = &[
    ("ansible_ssh_host", "ansible_host"),
    ("ansible_ssh_port", "ansible_port"),
    ("ansible_ssh_user", "ansible_user"),
    ("ansible_ssh_pass", "ansible_password"),
    ("ansible_ssh_sudo_pass", "ansible_become_password"),
    ("ansible_sudo_pass", "ansible_become_password"),
    ("ansible_become_pass", "ansible_become_password"),
    ("ansible_sudo", "ansible_become"),
    ("ansible_sudo_user", "ansible_become_user"),
    ("ansible_connection_type", "ansible_connection"),
    ("ansible_timeout", "ansible_ssh_timeout"),
    ("ansible_command_timeout", "rustsible_command_timeout"),
    (
        "ansible_ssh_host_key_checking",
        "rustsible_host_key_checking",
    ),
    ("ansible_host_key_checking", "rustsible_host_key_checking"),
    ("ansible_ssh_known_hosts_file", "rustsible_known_hosts_file"),
];

pub(crate) fn canonical_variable_name(name: &str) -> &str {
    VARIABLE_ALIASES
        .iter()
        .find_map(|(alias, canonical)| (*alias == name).then_some(*canonical))
        .unwrap_or(name)
}

/// Normalize public Ansible aliases inside each layer, before overlaying the
/// next layer. Otherwise an inherited modern spelling could defeat a host/task
/// override that uses an alias. Values remain typed and are never stringified.
pub(crate) fn merge_variable_layer(
    target: &mut HashMap<String, Value>,
    layer: &HashMap<String, Value>,
) {
    target.extend(layer.clone());
    for &(alias, canonical) in VARIABLE_ALIASES {
        if !layer.contains_key(canonical) {
            if let Some(value) = layer.get(alias) {
                target.insert(canonical.to_owned(), value.clone());
            }
        }
    }
}

impl Host {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.split(' ').next().unwrap_or(name).to_owned(),
            variables: HashMap::new(),
            inherited_variables: HashMap::new(),
            defaults: Default::default(),
        }
    }
    pub fn with_port(mut self, port: u16) -> Self {
        self.variables
            .insert("ansible_port".into(), Value::Number(port.into()));
        self
    }
    pub fn get_variable(&self, key: &str) -> Option<&Value> {
        self.variables
            .get(key)
            .or_else(|| self.inherited_variables.get(key))
            .or_else(|| self.defaults.get(key))
    }
    pub fn set_variable(&mut self, key: &str, value: &str) {
        self.set_typed_variable(key, Value::String(value.to_owned()));
    }
    pub(crate) fn set_typed_variable(&mut self, key: &str, value: Value) {
        self.variables.insert(key.to_owned(), value);
    }
    pub fn add_inherited_variable(&mut self, key: &str, value: &str) -> bool {
        if self.variables.contains_key(key) {
            return false;
        }
        self.add_typed_inherited_variable(key, &Value::String(value.to_owned()));
        true
    }
    pub(crate) fn add_typed_inherited_variable(&mut self, key: &str, value: &Value) {
        if !self.variables.contains_key(key) {
            self.inherited_variables
                .insert(key.to_owned(), value.clone());
        }
    }
    pub(crate) fn clear_inherited_variables(&mut self) {
        self.inherited_variables.clear();
    }
    pub fn all_variables(&self) -> HashMap<String, Value> {
        let mut result = (*self.defaults).clone();
        merge_variable_layer(&mut result, &self.inventory_variables());
        result
    }
    pub(crate) fn inventory_variables(&self) -> HashMap<String, Value> {
        let mut result = HashMap::new();
        merge_variable_layer(&mut result, &self.inherited_variables);
        merge_variable_layer(&mut result, &self.variables);
        result
    }
    fn connection_variable(&self, canonical: &str, alias: &str) -> Option<&Value> {
        self.variables
            .get(canonical)
            .or_else(|| self.variables.get(alias))
            .or_else(|| self.inherited_variables.get(canonical))
            .or_else(|| self.inherited_variables.get(alias))
            .or_else(|| self.defaults.get(canonical))
            .or_else(|| self.defaults.get(alias))
    }
    pub fn hostname(&self) -> Result<&str> {
        match self.connection_variable("ansible_host", "ansible_ssh_host") {
            Some(Value::String(value)) if !value.trim().is_empty() => Ok(value),
            Some(_) => bail!("ansible_host must be a non-empty string"),
            None => Ok(&self.name),
        }
    }
    pub fn port(&self) -> Result<u16> {
        match self.connection_variable("ansible_port", "ansible_ssh_port") {
            Some(Value::Number(value)) => value
                .as_u64()
                .and_then(|value| u16::try_from(value).ok())
                .filter(|port| *port != 0)
                .context("ansible_port must be between 1 and 65535"),
            Some(Value::String(value)) => value
                .parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .context("ansible_port must be between 1 and 65535"),
            Some(_) => bail!("ansible_port must be an integer or integer string"),
            None => Ok(22),
        }
    }
    pub fn get_ssh_user(&self) -> Option<&str> {
        self.connection_variable("ansible_user", "ansible_ssh_user")
            .and_then(Value::as_str)
    }
    pub fn get_ssh_password(&self) -> Option<&str> {
        self.connection_variable("ansible_password", "ansible_ssh_pass")
            .and_then(Value::as_str)
    }
    pub fn get_ssh_private_key(&self) -> Option<&str> {
        self.get_variable("ansible_ssh_private_key_file")
            .and_then(Value::as_str)
    }
    pub fn get_ssh_sudo_password(&self) -> Option<&str> {
        self.get_variable("ansible_become_password")
            .or_else(|| self.get_variable("ansible_become_pass"))
            .or_else(|| self.get_variable("ansible_sudo_pass"))
            .or_else(|| self.get_variable("ansible_ssh_sudo_pass"))
            .and_then(Value::as_str)
    }
    pub fn get_become(&self) -> Option<bool> {
        match self.connection_variable("ansible_become", "ansible_sudo")? {
            Value::Bool(value) => Some(*value),
            Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
                "true" | "yes" | "on" | "1" => Some(true),
                "false" | "no" | "off" | "0" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }
    pub fn get_become_user(&self) -> Option<&str> {
        self.connection_variable("ansible_become_user", "ansible_sudo_user")
            .and_then(Value::as_str)
    }
}
impl PartialEq for Host {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for Host {}

impl Hash for Host {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}

#[derive(Debug, Clone)]
pub struct HostGroup {
    pub name: String,
    pub hosts: HashSet<String>,
    pub variables: HashMap<String, Value>,
    pub parent: Option<String>,
    pub children: HashSet<String>,
}

impl HostGroup {
    pub fn new(name: &str) -> Self {
        debug!("Creating new host group: {}", name);
        HostGroup {
            name: name.to_string(),
            hosts: HashSet::new(),
            variables: HashMap::new(),
            parent: None,
            children: HashSet::new(),
        }
    }

    pub fn with_parent(mut self, parent: &str) -> Self {
        self.parent = Some(parent.to_string());
        self
    }

    pub fn add_child(&mut self, child: &str) -> bool {
        self.children.insert(child.to_string())
    }

    pub fn add_host(&mut self, host: &str) -> bool {
        self.hosts.insert(host.to_string())
    }

    pub fn add_variable(&mut self, key: &str, value: &str) {
        self.variables
            .insert(key.to_string(), Value::String(value.to_string()));
    }

    pub fn set_variable(&mut self, key: &str, value: &str) {
        self.variables
            .insert(key.to_string(), Value::String(value.to_string()));
    }

    pub(crate) fn set_typed_variable(&mut self, key: &str, value: Value) {
        self.variables.insert(key.to_string(), value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_new_and_port() {
        let host = Host::new("example.com");
        assert_eq!(host.name, "example.com");
        assert_eq!(host.hostname().unwrap(), "example.com");
        assert_eq!(host.port().unwrap(), 22);

        let host2 = Host::new("testhost extra");
        assert_eq!(host2.name, "testhost");
        assert_eq!(host2.hostname().unwrap(), "testhost");

        let host3 = host.with_port(2222);
        assert_eq!(host3.port().unwrap(), 2222);
    }

    #[test]
    fn test_set_and_get_variable() {
        let mut host = Host::new("h1");
        // Test setting normal variable
        host.set_variable("foo", "bar");
        assert_eq!(
            host.get_variable("foo"),
            Some(&Value::String("bar".to_string()))
        );
        // Inherited variable should not override explicit
        host.set_variable("baz", "qux");
        assert!(!host.add_inherited_variable("baz", "new"));

        // Test ansible_host changes hostname
        host.set_variable("ansible_host", "remote");
        assert_eq!(host.hostname().unwrap(), "remote");

        // Test ansible_port changes port
        host.set_variable("ansible_port", "2023");
        assert_eq!(host.port().unwrap(), 2023);
    }

    #[test]
    fn test_ssh_credentials_helpers() {
        let mut host = Host::new("h2");
        host.set_variable("ansible_user", "user1");
        host.set_variable("ansible_password", "pass1");
        host.set_variable("ansible_become_password", "become1");
        host.set_variable("ansible_ssh_private_key_file", "/key");

        assert_eq!(host.get_ssh_user(), Some("user1"));
        assert_eq!(host.get_ssh_password(), Some("pass1"));
        assert_eq!(host.get_ssh_sudo_password(), Some("become1"));
        assert_eq!(host.get_ssh_private_key(), Some("/key"));
    }
}
