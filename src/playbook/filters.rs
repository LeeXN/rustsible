use log::debug;
use sha_crypt::{sha256_crypt_b64, sha512_crypt_b64, Sha256Params, Sha512Params};
use tera::{Kwargs, State, TeraResult};

/// Password hash filter implementing the supported subset of Ansible's
/// `password_hash` functionality.
///
/// SHA-256 and SHA-512 are generated with their corresponding Unix crypt
/// implementations. A salt is deliberately required: choosing a fresh random
/// salt every time makes user-management tasks report a password change on
/// every run, while deriving one from the password would weaken the purpose of
/// salting. MD5-crypt and bcrypt are deliberately rejected until a compatible
/// crypt implementation is available; returning a string that only resembles
/// one of those formats would create unusable account passwords.
pub struct PasswordHashFilter;

impl PasswordHashFilter {
    fn call(&self, password: &str, args: &Kwargs) -> TeraResult<String> {
        let hash_type = args.get::<&str>("hash_type")?.unwrap_or("sha512");

        debug!(
            "PasswordHashFilter: password='***', hash_type='{}'",
            hash_type
        );

        let hashed = match hash_type {
            "sha512" => generate_sha512_hash(password, required_salt(args)?)?,
            "sha256" => generate_sha256_hash(password, required_salt(args)?)?,
            "md5" => return Err(disabled_hash_error("md5", "MD5-crypt")),
            "bcrypt" => return Err(disabled_hash_error("bcrypt", "bcrypt")),
            _ => return Err(unsupported_hash_error(hash_type)),
        };

        debug!("PasswordHashFilter: Generated hash for password");
        Ok(hashed)
    }
}

fn password_hash_filter(password: &str, args: Kwargs, _: &State) -> TeraResult<String> {
    PasswordHashFilter.call(password, &args)
}

/// Generate SHA-512 hash compatible with Linux systems
fn generate_sha512_hash(password: &str, salt: &str) -> TeraResult<String> {
    let params = Sha512Params::new(5000)
        .map_err(|e| tera::Error::message(format!("Failed to create SHA-512 params: {:?}", e)))?;

    let digest = sha512_crypt_b64(password.as_bytes(), salt.as_bytes(), &params)
        .map_err(|e| tera::Error::message(format!("SHA-512 hash generation failed: {:?}", e)))?;
    Ok(format!("$6${salt}${digest}"))
}

/// Generate SHA-256 hash
fn generate_sha256_hash(password: &str, salt: &str) -> TeraResult<String> {
    let params = Sha256Params::new(5000)
        .map_err(|e| tera::Error::message(format!("Failed to create SHA-256 params: {:?}", e)))?;

    let digest = sha256_crypt_b64(password.as_bytes(), salt.as_bytes(), &params)
        .map_err(|e| tera::Error::message(format!("SHA-256 hash generation failed: {:?}", e)))?;
    Ok(format!("$5${salt}${digest}"))
}

fn required_salt(args: &Kwargs) -> TeraResult<&str> {
    let salt = args.get::<&str>("salt")?.ok_or_else(|| {
        tera::Error::message(
            "password_hash requires an explicit salt so repeated runs produce the same hash",
        )
    })?;

    if salt.is_empty() || salt.len() > 16 {
        return Err(tera::Error::message(
            "password_hash salt must contain between 1 and 16 ASCII characters",
        ));
    }
    if !salt
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'/'))
    {
        return Err(tera::Error::message(
            "password_hash salt may contain only ASCII letters, digits, '.' and '/'",
        ));
    }

    Ok(salt)
}

fn disabled_hash_error(hash_type: &str, crypt_name: &str) -> tera::Error {
    tera::Error::message(format!(
        "Password hash type '{}' is disabled because a compatible {} implementation is not available; use 'sha256' or 'sha512'",
        hash_type, crypt_name
    ))
}

fn unsupported_hash_error(hash_type: &str) -> tera::Error {
    tera::Error::message(format!(
        "Unsupported password hash type '{}'; supported types are 'sha256' and 'sha512'",
        hash_type
    ))
}

/// Simplified selectattr filter (handles 'equalto' test, attempts YAML fallback)
pub struct SelectAttrFilter;
impl SelectAttrFilter {
    fn call(&self, value: &[tera::Value], args: &Kwargs) -> TeraResult<Vec<tera::Value>> {
        debug!("SelectAttrFilter: processing a sequence value");
        let key = args.must_get::<String>("key")?;
        let test = args.get::<&str>("test")?.unwrap_or("equalto");
        let expected_val = args
            .get::<tera::Value>("value")?
            .unwrap_or_else(tera::Value::none);
        debug!("SelectAttrFilter: key='{}', test='{}'", key, test);

        if test != "equalto" {
            return Err(tera::Error::message(format!(
                "selectattr test '{test}' is not supported; only 'equalto' is available"
            )));
        }

        let mut res = Vec::new();
        for (index, val) in value.iter().enumerate() {
            if val.get_from_path(&key) == Some(&expected_val) {
                debug!("SelectAttrFilter item[{}]: Match found!", index);
                res.push(val.clone());
            } else {
                debug!("SelectAttrFilter item[{}]: No match.", index);
            }
        }
        debug!("SelectAttrFilter: Filtered result size: {}", res.len());
        Ok(res)
    }
}

fn selectattr_filter(
    value: &[tera::Value],
    args: Kwargs,
    _: &State,
) -> TeraResult<Vec<tera::Value>> {
    SelectAttrFilter.call(value, &args)
}

/// Simplified map(attribute=...) filter (attempts YAML fallback)
pub struct MapAttributeFilter;
impl MapAttributeFilter {
    fn call(&self, value: &[tera::Value], args: &Kwargs) -> TeraResult<Vec<tera::Value>> {
        let attr = args.must_get::<String>("attribute")?;

        let mut res = Vec::new();
        for val in value {
            if let Some(found_value) = val.get_from_path(&attr) {
                res.push(found_value.clone());
            } else {
                debug!("MapAttributeFilter: attribute '{}' not found in item", attr);
            }
        }

        Ok(res)
    }
}

fn map_attribute_filter(
    value: &[tera::Value],
    args: Kwargs,
    _: &State,
) -> TeraResult<Vec<tera::Value>> {
    MapAttributeFilter.call(value, &args)
}

/// Helper function to register all custom filters to a Tera instance
pub fn register_ansible_filters(tera: &mut tera::Tera) {
    tera.register_filter("password_hash", password_hash_filter);
    tera.register_filter("selectattr", selectattr_filter);
    tera.register_filter("map", map_attribute_filter);
    tera.register_test("failed", result_failed);
    tera.register_test("succeeded", result_succeeded);
    tera.register_test("success", result_succeeded);
    tera.register_test("changed", result_changed);
    tera.register_test("skipped", result_skipped);
    debug!("Registered Ansible-compatible filters and result testers");
}

fn result_failed(value: &tera::Value, args: Kwargs, _: &State) -> TeraResult<bool> {
    result_flag("failed", value, &args)
}

fn result_succeeded(value: &tera::Value, args: Kwargs, _: &State) -> TeraResult<bool> {
    result_flag("failed", value, &args).map(|failed| !failed)
}

fn result_changed(value: &tera::Value, args: Kwargs, _: &State) -> TeraResult<bool> {
    result_flag("changed", value, &args)
}

fn result_skipped(value: &tera::Value, args: Kwargs, _: &State) -> TeraResult<bool> {
    result_flag("skipped", value, &args)
}

fn result_flag(flag: &str, value: &tera::Value, args: &Kwargs) -> TeraResult<bool> {
    if args.iter().next().is_some() {
        return Err(tera::Error::message(format!(
            "Result tester '{}' does not accept arguments",
            flag
        )));
    }
    Ok(value
        .get_from_path(flag)
        .and_then(tera::Value::as_bool)
        .unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha_crypt::{sha256_check, sha512_check};

    fn filter_args(pairs: &[(&'static str, &str)]) -> Kwargs {
        let mut map = tera::Map::new();
        for (key, value) in pairs {
            map.insert((*key).into(), tera::Value::from(*value));
        }
        Kwargs::new(std::sync::Arc::new(map))
    }

    #[test]
    fn test_password_hash_filter_sha512() {
        let filter = PasswordHashFilter {};
        let args = filter_args(&[("hash_type", "sha512"), ("salt", "stable.salt/123")]);

        let result = filter.call("testpassword", &args);
        assert!(result.is_ok());

        let binding = result.unwrap();
        let hash = binding.as_str();
        assert!(hash.starts_with("$6$stable.salt/123$"));
        assert!(hash.len() > 50); // Reasonable hash length
        assert!(sha512_check("testpassword", hash).is_ok());
        assert!(sha512_check("wrongpassword", hash).is_err());
        assert_eq!(filter.call("testpassword", &args).unwrap(), hash);
    }

    #[test]
    fn test_password_hash_filter_sha256() {
        let filter = PasswordHashFilter {};
        let args = filter_args(&[("hash_type", "sha256"), ("salt", "fixedSalt123")]);

        let result = filter.call("testpassword", &args);
        assert!(result.is_ok());

        let binding = result.unwrap();
        let hash = binding.as_str();
        assert!(hash.starts_with("$5$fixedSalt123$"));
        assert!(sha256_check("testpassword", hash).is_ok());
        assert!(sha256_check("wrongpassword", hash).is_err());
        assert_eq!(filter.call("testpassword", &args).unwrap(), hash);
    }

    #[test]
    fn test_password_hash_filter_requires_salt() {
        let filter = PasswordHashFilter {};
        let args = Kwargs::default();

        let error = filter.call("testpassword", &args).unwrap_err().to_string();
        assert!(error.contains("requires an explicit salt"));
        assert!(error.contains("repeated runs"));
    }

    #[test]
    fn test_password_hash_filter_defaults_to_sha512_with_named_salt() {
        let filter = PasswordHashFilter {};
        let args = filter_args(&[("salt", "defaultSalt")]);

        let binding = filter.call("testpassword", &args).unwrap();
        let hash = binding.as_str();
        assert!(hash.starts_with("$6$defaultSalt$"));
        assert!(sha512_check("testpassword", hash).is_ok());
    }

    #[test]
    fn test_password_hash_filter_is_deterministic_in_tera_template() {
        let mut tera = tera::Tera::default();
        register_ansible_filters(&mut tera);
        let mut context = tera::Context::new();
        context.insert("password", "testpassword");
        let template = "{{ password | password_hash(hash_type='sha512', salt='templateSalt') }}";

        let first = tera.render_str(template, &context, false).unwrap();
        let second = tera.render_str(template, &context, false).unwrap();

        assert_eq!(first, second);
        assert!(first.starts_with("$6$templateSalt$"));
        assert!(sha512_check("testpassword", &first).is_ok());
    }

    #[test]
    fn test_password_hash_filter_rejects_invalid_salt() {
        let filter = PasswordHashFilter {};
        for invalid_salt in ["", "contains$dollar", "way-too-long-for-crypt"] {
            let args = filter_args(&[("salt", invalid_salt)]);
            assert!(
                filter.call("testpassword", &args).is_err(),
                "{invalid_salt:?}"
            );
        }
    }

    #[test]
    fn test_password_hash_filter_rejects_md5() {
        let filter = PasswordHashFilter {};
        let args = filter_args(&[("hash_type", "md5")]);

        let error = filter.call("testpassword", &args).unwrap_err().to_string();
        assert!(error.contains("md5"));
        assert!(error.contains("disabled"));
        assert!(error.contains("sha256"));
    }

    #[test]
    fn test_password_hash_filter_rejects_bcrypt() {
        let filter = PasswordHashFilter {};
        let args = filter_args(&[("hash_type", "bcrypt")]);

        let error = filter.call("testpassword", &args).unwrap_err().to_string();
        assert!(error.contains("bcrypt"));
        assert!(error.contains("disabled"));
        assert!(error.contains("sha512"));
    }

    #[test]
    fn test_password_hash_filter_rejects_unknown_type() {
        let filter = PasswordHashFilter {};
        let args = filter_args(&[("hash_type", "plain")]);

        let error = filter.call("testpassword", &args).unwrap_err().to_string();
        assert!(error.contains("Unsupported password hash type 'plain'"));
        assert!(error.contains("sha256"));
        assert!(error.contains("sha512"));
    }

    #[test]
    fn selectattr_map_and_result_tests_render_with_tera_two() {
        let mut tera = tera::Tera::default();
        register_ansible_filters(&mut tera);
        let mut context = tera::Context::new();
        context.insert(
            "items",
            &serde_json::json!([
                {"name": "alpha", "enabled": true},
                {"name": "beta", "enabled": false},
                {"name": "gamma", "enabled": true}
            ]),
        );
        context.insert(
            "result",
            &serde_json::json!({"failed": true, "changed": false, "skipped": false}),
        );

        let selected = tera
            .render_str(
                "{{ items | selectattr(key='enabled', value=true) | map(attribute='name') | join(sep=',') }}",
                &context,
                false,
            )
            .unwrap();
        assert_eq!(selected, "alpha,gamma");

        let tested = tera
            .render_str(
                "{{ result is failed }}|{{ result is succeeded }}|{{ result is changed }}|{{ result is skipped }}",
                &context,
                false,
            )
            .unwrap();
        assert_eq!(tested, "true|false|false|false");
    }

    #[test]
    fn selectattr_rejects_unsupported_tests_instead_of_returning_unfiltered_input() {
        let mut tera = tera::Tera::default();
        register_ansible_filters(&mut tera);
        let mut context = tera::Context::new();
        context.insert("items", &serde_json::json!([{"name": "alpha"}]));

        let error = tera
            .render_str(
                "{{ items | selectattr(key='name', test='defined') }}",
                &context,
                false,
            )
            .unwrap_err();

        assert!(error.to_string().contains("selectattr test 'defined'"));
    }
}
