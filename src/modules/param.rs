use anyhow::Result;
use serde::de::DeserializeOwned;
use serde_yaml::Value;

/// Check if a parameter exists in a YAML mapping.
/// Returns true if the parameter exists, false otherwise.
#[allow(dead_code)]
pub fn has_param(args: &Value, name: &str) -> bool {
    match args {
        Value::Mapping(map) => map.contains_key(Value::String(name.to_string())),
        _ => false,
    }
}

/// Extract a required parameter of type T from a YAML mapping.
/// Returns an error if the parameter is missing or type does not match.
pub fn get_param<T: DeserializeOwned>(args: &Value, name: &str) -> Result<T> {
    match args {
        Value::Mapping(map) => {
            if let Some(val) = map.get(Value::String(name.to_string())) {
                serde_yaml::from_value(val.clone())
                    .map_err(|e| anyhow::anyhow!("Parameter '{}' type error: {}", name, e))
            } else {
                Err(anyhow::anyhow!("Missing required parameter: {}", name))
            }
        }
        _ => Err(anyhow::anyhow!("Arguments must be a mapping")),
    }
}

/// Extract an optional parameter of type T from a YAML mapping.
/// Returns None if the parameter is missing or type does not match.
pub fn get_optional_param<T: DeserializeOwned>(args: &Value, name: &str) -> Result<Option<T>> {
    let Value::Mapping(map) = args else {
        return Err(anyhow::anyhow!("Arguments must be a mapping"));
    };
    map.get(Value::String(name.to_string()))
        .map(|value| {
            serde_yaml::from_value(value.clone())
                .map_err(|error| anyhow::anyhow!("Parameter '{}' type error: {}", name, error))
        })
        .transpose()
}

/// Reject misspelled or silently unsupported parameters before a module can
/// report success without applying the requested semantics.
pub fn validate_params(args: &Value, allowed: &[&str]) -> Result<()> {
    let Value::Mapping(map) = args else {
        return Err(anyhow::anyhow!("Arguments must be a mapping"));
    };
    for key in map.keys() {
        let Value::String(key) = key else {
            return Err(anyhow::anyhow!("Module parameter names must be strings"));
        };
        if !allowed.contains(&key.as_str()) {
            return Err(anyhow::anyhow!("Unsupported module parameter: {}", key));
        }
    }
    Ok(())
}

/// Matches the fixed ansible-core 2.21 stat checksum choices. Keeping the
/// length table here also validates inline download digests before any IO.
pub(crate) fn digest_hex_length(algorithm: &str) -> Result<usize> {
    match algorithm {
        "md5" => Ok(32),
        "sha1" => Ok(40),
        "sha224" => Ok(56),
        "sha256" => Ok(64),
        "sha384" => Ok(96),
        "sha512" => Ok(128),
        _ => anyhow::bail!("Unsupported checksum algorithm: {algorithm}"),
    }
}

pub(crate) fn parse_mode(mode: &str) -> Result<u32> {
    if mode.is_empty() || !mode.bytes().all(|byte| matches!(byte, b'0'..=b'7')) {
        anyhow::bail!("File mode must be a non-empty octal string");
    }
    let mode = u32::from_str_radix(mode, 8).map_err(|_| anyhow::anyhow!("Invalid file mode"))?;
    if mode > 0o7777 {
        anyhow::bail!("File mode is outside the supported range");
    }
    Ok(mode)
}

/// Linux uid/gid use an unsigned 32-bit ID; all-ones is chown's "unchanged"
/// sentinel and must not be accepted as a requested identity.
pub(crate) fn validate_identity(value: &str) -> Result<()> {
    if value.is_empty() || value.contains(':') || value.chars().any(char::is_control) {
        anyhow::bail!(
            "File identity must be a non-empty name or ID without colons or control characters"
        );
    }
    if value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<u32>().map_or(true, |id| id == u32::MAX)
    {
        anyhow::bail!("Numeric file identity must be between 0 and 4294967294");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::{Mapping, Value};

    #[test]
    fn test_get_param_string_ok() {
        let mut map = Mapping::new();
        map.insert(
            Value::String("key".to_string()),
            Value::String("val".to_string()),
        );
        let args = Value::Mapping(map);
        assert_eq!(get_param::<String>(&args, "key").unwrap(), "val");
    }

    #[test]
    fn test_get_param_i64_ok() {
        let mut map = Mapping::new();
        map.insert(
            Value::String("num".to_string()),
            Value::Number(serde_yaml::Number::from(42)),
        );
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
        map.insert(
            Value::String("foo".to_string()),
            Value::Number(serde_yaml::Number::from(1)),
        );
        let args = Value::Mapping(map);
        let err = get_param::<String>(&args, "foo").unwrap_err();
        assert!(err.to_string().contains("type error"));
    }

    #[test]
    fn test_get_optional_param_some() {
        let mut map = Mapping::new();
        map.insert(
            Value::String("foo".to_string()),
            Value::String("bar".to_string()),
        );
        let args = Value::Mapping(map);
        assert_eq!(
            get_optional_param::<String>(&args, "foo").unwrap(),
            Some("bar".to_string())
        );
    }

    #[test]
    fn test_get_optional_param_none() {
        let map = Mapping::new();
        let args = Value::Mapping(map);
        assert_eq!(get_optional_param::<String>(&args, "none").unwrap(), None);
    }

    #[test]
    fn optional_parameter_type_errors_are_not_silently_ignored() {
        let mut map = Mapping::new();
        map.insert(Value::String("mode".into()), Value::Bool(true));
        assert!(get_optional_param::<String>(&Value::Mapping(map), "mode").is_err());
    }

    #[test]
    fn unsupported_parameters_are_rejected() {
        let mut map = Mapping::new();
        map.insert(Value::String("recurse".into()), Value::Bool(true));
        assert!(validate_params(&Value::Mapping(map), &["path", "state"]).is_err());
    }

    #[test]
    fn test_has_param_true() {
        let mut map = Mapping::new();
        map.insert(
            Value::String("foo".to_string()),
            Value::String("bar".to_string()),
        );
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

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;
    use serde_yaml::{Mapping, Value};

    proptest! {
        #[test]
        fn test_get_param_any_string(s in ".*") {
            let mut map = Mapping::new();
            map.insert(Value::String("key".to_string()), Value::String(s.clone()));
            let args = Value::Mapping(map);

            let result = get_param::<String>(&args, "key");
            prop_assert!(result.is_ok());
            prop_assert_eq!(result.unwrap(), s);
        }

        #[test]
        fn test_get_param_any_i64(n in any::<i64>()) {
            let mut map = Mapping::new();
            map.insert(Value::String("key".to_string()), Value::Number(serde_yaml::Number::from(n)));
            let args = Value::Mapping(map);

            let result = get_param::<i64>(&args, "key");
            prop_assert!(result.is_ok());
            prop_assert_eq!(result.unwrap(), n);
        }
    }
}
