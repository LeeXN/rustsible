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
pub use crate::digest_hex_length;

pub fn parse_mode(mode: &str) -> Result<u32> {
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
pub fn validate_identity(value: &str) -> Result<()> {
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
pub fn validate_variable_name(name: &str, label: &str) -> anyhow::Result<()> {
    let mut characters = name.chars();
    if !characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        || !characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        anyhow::bail!("{label} contains invalid variable name '{name}'");
    }
    Ok(())
}
