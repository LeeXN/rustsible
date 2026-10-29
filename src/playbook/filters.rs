use minijinja::value::{Kwargs, Value};
use minijinja::{Environment, Error, ErrorKind};
use sha_crypt::{sha256_crypt_b64, sha512_crypt_b64, Sha256Params, Sha512Params};

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidOperation, message.into())
}

/// An explicit stable salt makes account tasks idempotent without deriving a
/// predictable salt from the password. Unsupported crypt formats fail rather
/// than returning an unusable string that merely resembles a password hash.
fn password_hash(
    password: &str,
    hash_type: Option<&str>,
    salt: Option<&str>,
    kwargs: Kwargs,
) -> Result<String, Error> {
    let named_type: Option<&str> = kwargs.get("hash_type")?;
    let named_salt: Option<&str> = kwargs.get("salt")?;
    kwargs.assert_all_used()?;
    if (hash_type.is_some() && named_type.is_some()) || (salt.is_some() && named_salt.is_some()) {
        return Err(invalid(
            "password_hash received duplicate positional and named arguments",
        ));
    }
    let hash_type = hash_type.or(named_type).unwrap_or("sha512");
    let salt = salt.or(named_salt).ok_or_else(|| {
        invalid("password_hash requires an explicit salt so repeated runs produce the same hash")
    })?;
    if salt.is_empty()
        || salt.len() > 16
        || !salt
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'/'))
    {
        return Err(invalid(
            "password_hash salt must contain 1 to 16 ASCII letters, digits, '.' or '/'",
        ));
    }
    match hash_type {
        "sha512" => {
            let params = Sha512Params::new(5000)
                .map_err(|error| invalid(format!("Invalid SHA-512 parameters: {error:?}")))?;
            let hash = sha512_crypt_b64(password.as_bytes(), salt.as_bytes(), &params)
                .map_err(|error| invalid(format!("SHA-512 generation failed: {error:?}")))?;
            Ok(format!("$6${salt}${hash}"))
        }
        "sha256" => {
            let params = Sha256Params::new(5000)
                .map_err(|error| invalid(format!("Invalid SHA-256 parameters: {error:?}")))?;
            let hash = sha256_crypt_b64(password.as_bytes(), salt.as_bytes(), &params)
                .map_err(|error| invalid(format!("SHA-256 generation failed: {error:?}")))?;
            Ok(format!("$5${salt}${hash}"))
        }
        "md5" | "bcrypt" => Err(invalid(format!(
            "Password hash type '{hash_type}' is disabled; use sha256 or sha512"
        ))),
        _ => Err(invalid(format!(
            "Unsupported password hash type '{hash_type}'; use sha256 or sha512"
        ))),
    }
}

fn result_flag(value: Value, flag: &str) -> Result<bool, Error> {
    let value = value.get_attr(flag)?;
    Ok(value.kind() == minijinja::value::ValueKind::Bool && value.is_true())
}

pub fn register_ansible_filters(environment: &mut Environment<'_>) {
    environment.add_filter("password_hash", password_hash);
    environment.add_test("failed", |value: Value| result_flag(value, "failed"));
    environment.add_test("succeeded", |value: Value| {
        result_flag(value, "failed").map(|v| !v)
    });
    environment.add_test("success", |value: Value| {
        result_flag(value, "failed").map(|v| !v)
    });
    environment.add_test("changed", |value: Value| result_flag(value, "changed"));
    environment.add_test("skipped", |value: Value| result_flag(value, "skipped"));
    environment.add_test("finished", |value: Value| result_flag(value, "finished"));
}

#[cfg(test)]
mod tests {
    use minijinja::context;
    use sha_crypt::{sha256_check, sha512_check};

    #[test]
    fn password_hashes_verify_and_are_deterministic() {
        let environment = crate::playbook::templar::environment();
        for (kind, prefix) in [("sha256", "$5$"), ("sha512", "$6$")] {
            let positional =
                format!("{{{{ password | password_hash('{kind}', 'stable.salt/123') }}}}");
            let named = format!(
                "{{{{ password | password_hash(hash_type='{kind}', salt='stable.salt/123') }}}}"
            );
            let first = environment
                .render_str(&positional, context!(password => "testpassword"))
                .unwrap();
            let second = environment
                .render_str(&named, context!(password => "testpassword"))
                .unwrap();
            assert_eq!(first, second);
            assert!(first.starts_with(prefix));
            if kind == "sha256" {
                assert!(sha256_check("testpassword", &first).is_ok());
                assert!(sha256_check("wrong", &first).is_err());
            } else {
                assert!(sha512_check("testpassword", &first).is_ok());
                assert!(sha512_check("wrong", &first).is_err());
            }
        }
        assert!(environment
            .render_str(
                "{{ 'testpassword' | password_hash(salt='defaultSalt') }}",
                context!()
            )
            .unwrap()
            .starts_with("$6$"));
    }

    #[test]
    fn password_hash_rejects_invalid_and_unsupported_inputs() {
        let environment = crate::playbook::templar::environment();
        for expression in [
            "password_hash()",
            "password_hash(salt='')",
            "password_hash(salt='bad$salt')",
            "password_hash(salt='much-too-long-for-crypt')",
            "password_hash('md5', 'salt')",
            "password_hash('bcrypt', 'salt')",
            "password_hash('plain', 'salt')",
            "password_hash('sha512', 'salt', hash_type='sha256')",
            "password_hash(salt='salt', unknown=true)",
        ] {
            assert!(
                environment
                    .render_str(&format!("{{{{ 'password' | {expression} }}}}"), context!())
                    .is_err(),
                "{expression}"
            );
        }
    }

    #[test]
    fn result_tests_handle_missing_and_boolean_flags() {
        let environment = crate::playbook::templar::environment();
        assert_eq!(environment.render_str(
            "{{ result is failed }}|{{ result is succeeded }}|{{ result is success }}|{{ result is changed }}|{{ result is skipped }}",
            context!(result => context!(failed => true, changed => false)),
        ).unwrap(), "True|False|False|False|False");
    }
}
