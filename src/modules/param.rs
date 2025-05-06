use anyhow::Result;
use serde_yaml::Value;
use serde::de::DeserializeOwned;

/// Check if a parameter exists in a YAML mapping.
/// Returns true if the parameter exists, false otherwise.
#[allow(dead_code)]
pub fn has_param(args: &Value, name: &str) -> bool {
    match args {
        Value::Mapping(map) => map.contains_key(&Value::String(name.to_string())),
        _ => false,
    }
}

/// Extract a required parameter of type T from a YAML mapping.
/// Returns an error if the parameter is missing or type does not match.
pub fn get_param<T: DeserializeOwned>(args: &Value, name: &str) -> Result<T> {
    match args {
        Value::Mapping(map) => {
            if let Some(val) = map.get(&Value::String(name.to_string())) {
                serde_yaml::from_value(val.clone())
                    .map_err(|e| anyhow::anyhow!("Parameter '{}' type error: {} (value: {:?})", name, e, val))
            } else {
                Err(anyhow::anyhow!("Missing required parameter: {}", name))
            }
        },
        _ => Err(anyhow::anyhow!("Arguments must be a mapping")),
    }
}

/// Extract an optional parameter of type T from a YAML mapping.
/// Returns None if the parameter is missing or type does not match.
pub fn get_optional_param<T: DeserializeOwned>(args: &Value, name: &str) -> Option<T> {
    if let Value::Mapping(map) = args {
        if let Some(val) = map.get(&Value::String(name.to_string())) {
            serde_yaml::from_value(val.clone()).ok()
        } else {
            None
        }
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::{Value, Mapping};

    #[test]
    fn test_get_param_string_ok() {
        let mut map = Mapping::new();
        map.insert(Value::String("key".to_string()), Value::String("val".to_string()));
        let args = Value::Mapping(map);
        assert_eq!(get_param::<String>(&args, "key").unwrap(), "val");
    }

    #[test]
    fn test_get_param_i64_ok() {
        let mut map = Mapping::new();
        map.insert(Value::String("num".to_string()), Value::Number(serde_yaml::Number::from(42)));
        let args = Value::Mapping(map);
        assert_eq!(get_param::<i64>(&args, "num").unwrap(), 42);
    }

    #[test]
    fn test_get_param_missing() {
        let map = Mapping::new();
        let args = Value::Mapping(map);
        assert!(get_param::<String>(&args, "notfound").is_err());
    }

    #[test]
    fn test_get_param_type_error() {
        let mut map = Mapping::new();
        map.insert(Value::String("foo".to_string()), Value::Number(serde_yaml::Number::from(1)));
        let args = Value::Mapping(map);
        let err = get_param::<String>(&args, "foo").unwrap_err();
        assert!(err.to_string().contains("type error"));
    }

    #[test]
    fn test_get_optional_param_some() {
        let mut map = Mapping::new();
        map.insert(Value::String("foo".to_string()), Value::String("bar".to_string()));
        let args = Value::Mapping(map);
        assert_eq!(get_optional_param::<String>(&args, "foo"), Some("bar".to_string()));
    }

    #[test]
    fn test_get_optional_param_none() {
        let map = Mapping::new();
        let args = Value::Mapping(map);
        assert_eq!(get_optional_param::<String>(&args, "none"), None);
    }

    #[test]
    fn test_has_param_true() {
        let mut map = Mapping::new();
        map.insert(Value::String("foo".to_string()), Value::String("bar".to_string()));
        let args = Value::Mapping(map);
        assert!(has_param(&args, "foo"));
    }

    #[test]
    fn test_has_param_false() {
        let map = Mapping::new();
        let args = Value::Mapping(map);
        assert!(!has_param(&args, "foo"));
    }
} 