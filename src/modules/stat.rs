use anyhow::{Context, Result};
use serde_yaml::Value;
use std::collections::HashMap;

use crate::inventory::Host;
use crate::modules::param::{get_optional_param, get_param, validate_params};
use crate::modules::python::execute_json_mapping;
use crate::modules::ModuleResult;
use crate::ssh::connection::{Connection, SshConnection};

const STAT_SCRIPT: &str = r#"import hashlib
import json
import os
import stat as stat_module
import sys

path, algorithm = sys.argv[1], sys.argv[2]
result = {"exists": False}
try:
    metadata = os.lstat(path)
except FileNotFoundError:
    pass
else:
    result["exists"] = True
    result["isreg"] = stat_module.S_ISREG(metadata.st_mode)
    if result["isreg"]:
        try:
            digest = hashlib.new(algorithm)
        except ValueError as error:
            raise RuntimeError(f"unsupported checksum algorithm: {algorithm}") from error
        with open(path, "rb") as source:
            for chunk in iter(lambda: source.read(1024 * 1024), b""):
                digest.update(chunk)
        result["checksum"] = digest.hexdigest()

print(json.dumps({"stat": result}))
"#;

pub fn execute(
    connection: &dyn SshConnection,
    args: &Value,
    use_become: bool,
    become_user: &str,
    _check_mode: bool,
) -> Result<ModuleResult> {
    validate_params(args, &["path", "checksum_algorithm"])?;
    let path = get_param::<String>(args, "path")?;
    if path.is_empty() {
        anyhow::bail!("Stat path cannot be empty");
    }
    let algorithm =
        get_optional_param::<String>(args, "checksum_algorithm")?.unwrap_or_else(|| "sha1".into());
    let output = execute_json_mapping(
        connection,
        STAT_SCRIPT,
        &[path, algorithm],
        use_become,
        become_user,
    )?;
    let stat = output
        .get(Value::String("stat".to_string()))
        .cloned()
        .context("Stat module did not return stat data")?;
    let mut values = HashMap::new();
    values.insert("stat".to_string(), stat);

    Ok(ModuleResult {
        changed: false,
        msg: "Path inspected".to_string(),
        values,
        ..ModuleResult::default()
    })
}

pub fn execute_adhoc(
    host: &Host,
    args: &Value,
    use_become: bool,
    become_user: &str,
    check_mode: bool,
) -> Result<ModuleResult> {
    let connection = Connection::connect(host)?;
    execute(
        connection.as_connection(),
        args,
        use_become,
        become_user,
        check_mode,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::Host;
    use crate::ssh::connection::LocalConnection;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn returns_ansible_compatible_exists_and_checksum_fields() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("artifact");
        fs::write(&path, b"hello").unwrap();
        let mut host = Host::new("localhost");
        host.set_variable("ansible_connection", "local");
        let connection = LocalConnection::new(&host).unwrap();
        let args: Value = serde_yaml::from_str(&format!(
            "path: {}\nchecksum_algorithm: sha1\n",
            path.display()
        ))
        .unwrap();

        let result = execute(&connection, &args, false, "root", false).unwrap();
        let stat = result.values["stat"].as_mapping().unwrap();
        assert_eq!(
            stat.get(Value::String("exists".to_string())),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            stat.get(Value::String("checksum".to_string())),
            Some(&Value::String(
                "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d".to_string()
            ))
        );

        fs::remove_file(&path).unwrap();
        let missing = execute(&connection, &args, false, "root", false).unwrap();
        let missing_stat = missing.values["stat"].as_mapping().unwrap();
        assert_eq!(
            missing_stat.get(Value::String("exists".to_string())),
            Some(&Value::Bool(false))
        );
        assert!(!missing_stat.contains_key(Value::String("checksum".to_string())));
    }
}
