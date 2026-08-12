use log::debug;
use serde_yaml::Value;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone)]
pub struct Host {
    pub name: String,
    pub hostname: String,
    pub port: u16,
    pub variables: HashMap<String, String>,
    pub inherited_variables: HashMap<String, String>,
    /// Typed forms preserve YAML booleans, numbers, sequences, and mappings
    /// for templating and loops while the string maps remain compatible with
    /// connection settings and the public inventory-debug output.
    pub typed_variables: HashMap<String, Value>,
    pub typed_inherited_variables: HashMap<String, Value>,
}

impl Host {
    pub fn new(name: &str) -> Self {
        let cleaned_name = if let Some(space_pos) = name.find(' ') {
            let name_part = &name[0..space_pos];
            debug!("Cleaned hostname '{}' from '{}'", name_part, name);
            name_part
        } else {
            name
        };

        debug!("Creating new host: {}", cleaned_name);

        Host {
            name: cleaned_name.to_string(),
            hostname: cleaned_name.to_string(),
            port: 22,
            variables: HashMap::new(),
            inherited_variables: HashMap::new(),
            typed_variables: HashMap::new(),
            typed_inherited_variables: HashMap::new(),
        }
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self.variables
            .insert("ansible_port".to_string(), port.to_string());
        self.typed_variables
            .insert("ansible_port".to_string(), Value::Number(port.into()));
        self
    }

    pub fn add_inherited_variable(&mut self, key: &str, value: &str) -> bool {
        if !self.variables.contains_key(key) {
            self.inherited_variables
                .insert(key.to_string(), value.to_string());
            self.typed_inherited_variables
                .insert(key.to_string(), Value::String(value.to_string()));
            self.refresh_connection_fields();
            true
        } else {
            false
        }
    }

    pub(crate) fn clear_inherited_variables(&mut self) {
        self.inherited_variables.clear();
        self.typed_inherited_variables.clear();
        self.refresh_connection_fields();
    }

    pub fn get_variable(&self, key: &str) -> Option<&String> {
        self.variables
            .get(key)
            .or_else(|| self.inherited_variables.get(key))
    }

    pub fn set_variable(&mut self, key: &str, value: &str) {
        // Inventory values commonly contain passwords, tokens, or private-key
        // material. Never emit a value here; callers can log non-sensitive,
        // typed connection settings explicitly when needed.
        debug!(
            "Setting inventory variable '{}' for host {}",
            key, self.name
        );

        self.variables.insert(key.to_string(), value.to_string());
        self.typed_variables
            .insert(key.to_string(), Value::String(value.to_string()));
        self.refresh_connection_fields();
    }

    pub(crate) fn set_typed_variable(&mut self, key: &str, value: Value) {
        self.variables
            .insert(key.to_string(), inventory_value_text(&value));
        self.typed_variables.insert(key.to_string(), value);
        self.refresh_connection_fields();
    }

    pub(crate) fn add_typed_inherited_variable(&mut self, key: &str, value: &Value) {
        if !self.variables.contains_key(key) {
            self.inherited_variables
                .insert(key.to_string(), inventory_value_text(value));
            self.typed_inherited_variables
                .insert(key.to_string(), value.clone());
            self.refresh_connection_fields();
        }
    }

    /// Recompute typed connection fields from inventory variables.
    ///
    /// Host variables have precedence over inherited group variables.  The
    /// modern Ansible spelling also has precedence over its legacy `ssh`
    /// alias, independent of hash-map iteration order.
    fn refresh_connection_fields(&mut self) {
        self.hostname = self
            .variables
            .get("ansible_host")
            .or_else(|| self.variables.get("ansible_ssh_host"))
            .or_else(|| self.inherited_variables.get("ansible_host"))
            .or_else(|| self.inherited_variables.get("ansible_ssh_host"))
            .cloned()
            .unwrap_or_else(|| self.name.clone());

        self.port = self
            .variables
            .get("ansible_port")
            .or_else(|| self.variables.get("ansible_ssh_port"))
            .or_else(|| self.inherited_variables.get("ansible_port"))
            .or_else(|| self.inherited_variables.get("ansible_ssh_port"))
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|port| *port != 0)
            .unwrap_or(22);
    }

    pub fn get_ssh_user(&self) -> Option<&String> {
        self.get_variable("ansible_user")
            .or_else(|| self.get_variable("ansible_ssh_user"))
    }

    pub fn get_ssh_password(&self) -> Option<&String> {
        self.get_variable("ansible_password")
            .or_else(|| self.get_variable("ansible_ssh_pass"))
    }
    pub fn get_ssh_sudo_password(&self) -> Option<&String> {
        self.get_variable("ansible_become_password")
            .or_else(|| self.get_variable("ansible_become_pass"))
            .or_else(|| self.get_variable("ansible_sudo_pass"))
            .or_else(|| self.get_variable("ansible_ssh_sudo_pass"))
    }

    pub fn get_become(&self) -> Option<bool> {
        self.get_variable("ansible_become")
            .or_else(|| self.get_variable("ansible_sudo"))
            .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
                "1" | "yes" | "true" | "on" => Some(true),
                "0" | "no" | "false" | "off" => Some(false),
                _ => None,
            })
    }

    pub fn get_become_user(&self) -> Option<&String> {
        self.get_variable("ansible_become_user")
            .or_else(|| self.get_variable("ansible_sudo_user"))
    }

    pub fn get_ssh_private_key(&self) -> Option<&String> {
        self.get_variable("ansible_ssh_private_key_file")
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
    pub variables: HashMap<String, String>,
    pub typed_variables: HashMap<String, Value>,
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
            typed_variables: HashMap::new(),
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
        self.variables.insert(key.to_string(), value.to_string());
        self.typed_variables
            .insert(key.to_string(), Value::String(value.to_string()));
    }

    pub fn set_variable(&mut self, key: &str, value: &str) {
        self.variables.insert(key.to_string(), value.to_string());
        self.typed_variables
            .insert(key.to_string(), Value::String(value.to_string()));
    }

    pub(crate) fn set_typed_variable(&mut self, key: &str, value: Value) {
        self.variables
            .insert(key.to_string(), inventory_value_text(&value));
        self.typed_variables.insert(key.to_string(), value);
    }
}

fn inventory_value_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Sequence(_) | Value::Mapping(_) | Value::Tagged(_) => serde_yaml::to_string(value)
            .map(|value| value.trim().to_string())
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_new_and_port() {
        let host = Host::new("example.com");
        assert_eq!(host.name, "example.com");
        assert_eq!(host.hostname, "example.com");
        assert_eq!(host.port, 22);

        let host2 = Host::new("testhost extra");
        assert_eq!(host2.name, "testhost");
        assert_eq!(host2.hostname, "testhost");

        let host3 = host.with_port(2222);
        assert_eq!(host3.port, 2222);
    }

    #[test]
    fn test_set_and_get_variable() {
        let mut host = Host::new("h1");
        // Test setting normal variable
        host.set_variable("foo", "bar");
        assert_eq!(host.get_variable("foo"), Some(&"bar".to_string()));
        // Inherited variable should not override explicit
        host.set_variable("baz", "qux");
        assert!(!host.add_inherited_variable("baz", "new"));

        // Test ansible_host changes hostname
        host.set_variable("ansible_host", "remote");
        assert_eq!(host.hostname, "remote");

        // Test ansible_port changes port
        host.set_variable("ansible_port", "2023");
        assert_eq!(host.port, 2023);
    }

    #[test]
    fn test_ssh_credentials_helpers() {
        let mut host = Host::new("h2");
        host.set_variable("ansible_user", "user1");
        host.set_variable("ansible_password", "pass1");
        host.set_variable("ansible_become_password", "become1");
        host.set_variable("ansible_ssh_private_key_file", "/key");

        assert_eq!(host.get_ssh_user(), Some(&"user1".to_string()));
        assert_eq!(host.get_ssh_password(), Some(&"pass1".to_string()));
        assert_eq!(host.get_ssh_sudo_password(), Some(&"become1".to_string()));
        assert_eq!(host.get_ssh_private_key(), Some(&"/key".to_string()));
    }
}
