//! Shared testing utilities for Rustsible unit tests
//!
//! This module provides common helpers, mocks, and fixtures to reduce duplication
//! and ensure consistent test patterns across the codebase.

use anyhow::Result;
use serde_yaml::Value;

/// Common test result type for module tests
pub type ModuleTestResult = Result<(bool, String)>;

/// Create a mock inventory host with basic configuration
pub fn create_test_host(
    name: &str,
    hostname: &str,
    port: u16,
    user: Option<&str>,
    password: Option<&str>,
) -> crate::inventory::Host {
    let mut host = crate::inventory::Host::new(name).with_port(port);

    host.set_variable("ansible_host", hostname);

    if let Some(u) = user {
        host.set_variable("ansible_ssh_user", u);
    }

    if let Some(p) = password {
        host.set_variable("ansible_ssh_pass", p);
    }

    host
}

/// Create a test YAML mapping from key-value pairs
pub fn create_test_mapping(pairs: Vec<(&str, Value)>) -> Value {
    let mut map = serde_yaml::Mapping::new();
    for (key, value) in pairs {
        map.insert(Value::String(key.to_string()), value);
    }
    Value::Mapping(map)
}

/// Assert that two YAML values are equal, with better error messages
pub fn assert_yaml_eq(actual: &Value, expected: &Value) {
    if actual != expected {
        panic!(
            "YAML values not equal:\n  expected: {:#?}\n  actual:   {:#?}",
            expected, actual
        );
    }
}

/// Extract the host's SSH user variable, returns Option<String>
pub fn get_host_ssh_user(host: &crate::inventory::Host) -> Option<String> {
    host.get_ssh_user().map(|s| s.clone())
}

/// Extract the host's SSH password variable, returns Option<String>
pub fn get_host_ssh_password(host: &crate::inventory::Host) -> Option<String> {
    host.get_ssh_password().map(|s| s.clone())
}

/// Extract the host's SSH private key path variable, returns Option<String>
pub fn get_host_ssh_private_key(host: &crate::inventory::Host) -> Option<String> {
    host.get_ssh_private_key().map(|s| s.clone())
}

/// Extract the host's sudo password variable, returns Option<String>
pub fn get_host_ssh_sudo_password(host: &crate::inventory::Host) -> Option<String> {
    host.get_ssh_sudo_password().map(|s| s.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_test_host() {
        let host = create_test_host("test", "example.com", 22, Some("user"), Some("pass"));

        assert_eq!(host.name, "test");
        assert_eq!(host.hostname, "example.com");
        assert_eq!(host.port, 22);
        assert_eq!(get_host_ssh_user(&host), Some("user".to_string()));
        assert_eq!(get_host_ssh_password(&host), Some("pass".to_string()));
    }

    #[test]
    fn test_create_test_mapping() {
        let mapping = create_test_mapping(vec![
            ("key1", Value::String("value1".to_string())),
            ("key2", Value::Number(serde_yaml::Number::from(42))),
        ]);

        assert_yaml_eq(
            &mapping,
            &create_test_mapping(vec![
                ("key1", Value::String("value1".to_string())),
                ("key2", Value::Number(serde_yaml::Number::from(42))),
            ]),
        );
    }

    #[test]
    fn test_assert_yaml_eq_success() {
        let val1 = Value::String("test".to_string());
        let val2 = Value::String("test".to_string());
        assert_yaml_eq(&val1, &val2);
    }

    #[test]
    #[should_panic(expected = "YAML values not equal")]
    fn test_assert_yaml_eq_failure() {
        let val1 = Value::String("test".to_string());
        let val2 = Value::String("different".to_string());
        assert_yaml_eq(&val1, &val2);
    }
}
