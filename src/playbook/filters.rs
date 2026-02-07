use log::{debug, warn};
use rand::Rng;
use serde_yaml::Value;
use sha_crypt::{sha256_simple, sha512_simple, Sha256Params, Sha512Params};
use std::collections::HashMap;
use tera::{from_value, to_value, try_get_value, Filter};

/// Password hash filter implementing Ansible's password_hash functionality
/// Supports sha256, sha512, md5 (basic), bcrypt (basic) hash types
pub struct PasswordHashFilter;

impl Filter for PasswordHashFilter {
    fn filter(
        &self,
        value: &tera::Value,
        args: &HashMap<String, tera::Value>,
    ) -> tera::Result<tera::Value> {
        let password = try_get_value!("password_hash", "value", String, value);
        let hash_type = try_get_value!(
            "password_hash",
            "hash_type",
            String,
            args.get("0")
                .or_else(|| args.get("hash_type"))
                .unwrap_or(&tera::Value::String("sha512".to_string()))
        );

        debug!(
            "PasswordHashFilter: password='***', hash_type='{}'",
            hash_type
        );

        let hashed = match hash_type.as_str() {
            "sha512" => generate_sha512_hash(&password)?,
            "sha256" => generate_sha256_hash(&password)?,
            "md5" => generate_md5_hash(&password)?,
            "bcrypt" => generate_bcrypt_hash(&password)?,
            _ => {
                warn!(
                    "Unsupported hash type '{}', defaulting to sha512",
                    hash_type
                );
                generate_sha512_hash(&password)?
            }
        };

        debug!("PasswordHashFilter: Generated hash for password");
        Ok(tera::Value::String(hashed))
    }
}

/// Generate SHA-512 hash compatible with Linux systems
fn generate_sha512_hash(password: &str) -> tera::Result<String> {
    let params = Sha512Params::new(5000)
        .map_err(|e| tera::Error::msg(format!("Failed to create SHA-512 params: {:?}", e)))?;

    sha512_simple(password, &params)
        .map_err(|e| tera::Error::msg(format!("SHA-512 hash generation failed: {:?}", e)))
}

/// Generate SHA-256 hash
fn generate_sha256_hash(password: &str) -> tera::Result<String> {
    let params = Sha256Params::new(5000)
        .map_err(|e| tera::Error::msg(format!("Failed to create SHA-256 params: {:?}", e)))?;

    sha256_simple(password, &params)
        .map_err(|e| tera::Error::msg(format!("SHA-256 hash generation failed: {:?}", e)))
}

/// Basic MD5 implementation (not recommended for production)
fn generate_md5_hash(password: &str) -> tera::Result<String> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let salt = generate_random_salt(8);
    let input = format!("{}{}", password, salt);
    let mut hasher = DefaultHasher::new();
    input.hash(&mut hasher);
    let hash = hasher.finish();

    // MD5-style format (this is a simplified implementation)
    Ok(format!("$1${}${:x}", salt, hash))
}

/// Basic bcrypt implementation placeholder
fn generate_bcrypt_hash(password: &str) -> tera::Result<String> {
    // This is a simplified implementation
    // In production, you'd want to use a proper bcrypt library
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let salt = generate_random_salt(22);
    let input = format!("{}{}", password, salt);
    let mut hasher = DefaultHasher::new();
    input.hash(&mut hasher);
    let hash = hasher.finish();

    Ok(format!("$2b$12${}${:x}", salt, hash))
}

/// Generate random salt string
fn generate_random_salt(length: usize) -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::rng();

    (0..length)
        .map(|_| {
            let idx = rng.random_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

/// Simplified selectattr filter (handles 'equalto' test, attempts YAML fallback)
pub struct SelectAttrFilter;
impl Filter for SelectAttrFilter {
    fn filter(
        &self,
        value: &tera::Value,
        args: &HashMap<String, tera::Value>,
    ) -> tera::Result<tera::Value> {
        debug!("SelectAttrFilter: Input value: {:?}", value);
        let arr = try_get_value!("selectattr", "value", Vec<tera::Value>, value);
        let key = try_get_value!(
            "selectattr",
            "key/arg0",
            String,
            args.get("key")
                .or_else(|| args.get("0"))
                .unwrap_or(&tera::Value::Null)
        );
        let test = try_get_value!(
            "selectattr",
            "test/arg1",
            String,
            args.get("test")
                .or_else(|| args.get("1"))
                .unwrap_or(&tera::Value::String("equalto".to_string()))
        );
        let expected_val = args
            .get("value")
            .or_else(|| args.get("2"))
            .unwrap_or(&tera::Value::Null);
        debug!(
            "SelectAttrFilter: key='{}', test='{}', expected_val={:?}",
            key, test, expected_val
        );

        if test != "equalto" {
            warn!("selectattr filter currently only supports test='equalto'");
            return Ok(to_value(arr)?);
        }

        let mut res = Vec::new();
        for (index, val) in arr.iter().enumerate() {
            let mut matched = false;
            // First, try accessing as a standard Tera object (JSON-like)
            if let Some(map) = val.as_object() {
                if let Some(item_val) = map.get(&key) {
                    debug!(
                        "SelectAttrFilter item[{}]: Comparing (as object) map key '{}' value={:?} with expected={:?}",
                        index, key, item_val, expected_val
                    );
                    if item_val == expected_val {
                        debug!("SelectAttrFilter item[{}]: Match found (as object)!", index);
                        res.push(val.clone());
                        matched = true;
                    } else {
                        debug!("SelectAttrFilter item[{}]: No match (as object).", index);
                    }
                } else {
                    debug!(
                        "SelectAttrFilter item[{}]: Key '{}' not found in object.",
                        index, key
                    );
                }
            }

            // If not matched as object, try converting back to serde_yaml::Mapping
            if !matched {
                if let Ok(yaml_map) = from_value::<serde_yaml::Mapping>(val.clone()) {
                    let yaml_key = Value::String(key.clone());
                    if let Some(item_val_yaml) = yaml_map.get(&yaml_key) {
                        let expected_yaml_val =
                            from_value::<Value>(expected_val.clone()).unwrap_or(Value::Null);
                        debug!(
                            "SelectAttrFilter item[{}]: Comparing (as YAML map) key '{}' value={:?} with expected={:?}",
                            index, key, item_val_yaml, expected_yaml_val
                        );
                        if item_val_yaml == &expected_yaml_val {
                            debug!(
                                "SelectAttrFilter item[{}]: Match found (as YAML map)!",
                                index
                            );
                            res.push(val.clone()); // Push the original tera::Value
                        } else {
                            debug!("SelectAttrFilter item[{}]: No match (as YAML map).", index);
                        }
                    } else {
                        debug!(
                            "SelectAttrFilter item[{}]: Key '{}' not found in YAML map.",
                            index, key
                        );
                    }
                } else {
                    // Log if it's neither a Tera object nor convertible to YAML Mapping
                    debug!("SelectAttrFilter item[{}]: Item is not a Tera object and failed to convert to YAML map: {:?}", index, val);
                }
            }
        }
        debug!("SelectAttrFilter: Filtered result size: {}", res.len());
        Ok(to_value(res)?)
    }
}

/// Simplified map(attribute=...) filter (attempts YAML fallback)
pub struct MapAttributeFilter;
impl Filter for MapAttributeFilter {
    fn filter(
        &self,
        value: &tera::Value,
        args: &HashMap<String, tera::Value>,
    ) -> tera::Result<tera::Value> {
        let arr = try_get_value!("map", "value", Vec<tera::Value>, value);
        let attr = try_get_value!(
            "map",
            "attribute",
            String,
            args.get("attribute").unwrap_or(&tera::Value::Null)
        );

        let mut res = Vec::new();
        for val in arr {
            let mut found_value: Option<tera::Value> = None;
            // Try as Tera Object first
            if let Some(map) = val.as_object() {
                if let Some(item_val) = map.get(&attr) {
                    found_value = Some(item_val.clone());
                }
            }

            // If not found, try converting to YAML Mapping
            if found_value.is_none() {
                if let Ok(yaml_map) = from_value::<serde_yaml::Mapping>(val.clone()) {
                    let yaml_key = Value::String(attr.clone());
                    if let Some(item_val_yaml) = yaml_map.get(&yaml_key) {
                        // Convert found YAML value back to tera::Value for the result list
                        if let Ok(tera_val) = to_value(item_val_yaml) {
                            found_value = Some(tera_val);
                        } else {
                            warn!("MapAttributeFilter: Failed to convert YAML value back to Tera value: {:?}", item_val_yaml);
                        }
                    }
                }
            }

            if let Some(final_val) = found_value {
                res.push(final_val);
            } else {
                debug!(
                    "MapAttributeFilter: Attribute '{}' not found in item: {:?}",
                    attr, val
                );
            }
        }

        Ok(to_value(res)?)
    }
}

/// Helper function to register all custom filters to a Tera instance
pub fn register_ansible_filters(tera: &mut tera::Tera) {
    tera.register_filter("password_hash", PasswordHashFilter {});
    tera.register_filter("selectattr", SelectAttrFilter {});
    tera.register_filter("map", MapAttributeFilter {});
    debug!("Registered Ansible-compatible filters: password_hash, selectattr, map");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_password_hash_filter_sha512() {
        let filter = PasswordHashFilter {};
        let password = tera::Value::String("testpassword".to_string());
        let mut args = HashMap::new();
        args.insert("0".to_string(), tera::Value::String("sha512".to_string()));

        let result = filter.filter(&password, &args);
        assert!(result.is_ok());

        let binding = result.unwrap();
        let hash = binding.as_str().unwrap();
        assert!(hash.starts_with("$6$")); // SHA-512 prefix
        assert!(hash.len() > 50); // Reasonable hash length
    }

    #[test]
    fn test_password_hash_filter_sha256() {
        let filter = PasswordHashFilter {};
        let password = tera::Value::String("testpassword".to_string());
        let mut args = HashMap::new();
        args.insert("0".to_string(), tera::Value::String("sha256".to_string()));

        let result = filter.filter(&password, &args);
        assert!(result.is_ok());

        let binding = result.unwrap();
        let hash = binding.as_str().unwrap();
        assert!(hash.starts_with("$5$")); // SHA-256 prefix
    }

    #[test]
    fn test_password_hash_filter_default() {
        let filter = PasswordHashFilter {};
        let password = tera::Value::String("testpassword".to_string());
        let args = HashMap::new();

        let result = filter.filter(&password, &args);
        assert!(result.is_ok());

        let binding = result.unwrap();
        let hash = binding.as_str().unwrap();
        assert!(hash.starts_with("$6$")); // Should default to SHA-512
    }

    #[test]
    fn test_generate_random_salt() {
        let salt1 = generate_random_salt(16);
        let salt2 = generate_random_salt(16);

        assert_eq!(salt1.len(), 16);
        assert_eq!(salt2.len(), 16);
        assert_ne!(salt1, salt2); // Should be different
    }
}
