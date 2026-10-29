pub(crate) use rustsible_protocol::param::{digest_hex_length, parse_mode, validate_identity};
pub use rustsible_protocol::param::{get_optional_param, get_param, has_param, validate_params};

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
