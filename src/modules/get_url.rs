use anyhow::{bail, Context, Result};
use serde_yaml::Value;
use std::collections::HashMap;

use crate::inventory::Host;
use crate::modules::param::{get_optional_param, get_param, validate_params};
use crate::modules::python::execute_json_mapping;
use crate::modules::ModuleResult;
use crate::ssh::connection::{Connection, SshConnection};

const GET_URL_SCRIPT: &str = r#"import grp
import hashlib
import json
import os
import pwd
import sys
import tempfile
import urllib.parse
import urllib.request

p = json.loads(sys.argv[1])
url, dest = p["url"], p["dest"]
scheme = urllib.parse.urlparse(url).scheme.lower()
if scheme not in ("http", "https"):
    raise RuntimeError(f"unsupported URL scheme: {scheme or '<missing>'}")

def digest_file(path, algorithm):
    digest = hashlib.new(algorithm)
    with open(path, "rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()

def desired_identity(value, resolver):
    if value is None:
        return -1
    return int(value) if str(value).isdigit() else resolver(value)

if p["check_mode"]:
    request = urllib.request.Request(url, method="HEAD")
    with urllib.request.urlopen(request, timeout=30):
        pass
    print(json.dumps({"changed": True}))
    raise SystemExit(0)

directory = os.path.dirname(dest) or "."
if not os.path.isdir(directory):
    raise RuntimeError(f"destination directory does not exist: {directory}")

checksum = p.get("checksum")
checksum_algorithm = checksum_value = None
if checksum:
    checksum_algorithm, separator, checksum_value = checksum.partition(":")
    if not separator or not checksum_value or "://" in checksum_value:
        raise RuntimeError("checksum must use <algorithm>:<digest> format")
    hashlib.new(checksum_algorithm)

existing = os.path.isfile(dest)
if existing and checksum_value and not p["force"]:
    if digest_file(dest, checksum_algorithm) == checksum_value.lower():
        source_sha1 = digest_file(dest, "sha1")
        content_changed = False
        temporary = None
    else:
        temporary = "download"
else:
    temporary = "download"

if temporary:
    descriptor, temporary = tempfile.mkstemp(prefix=".rustsible-get-url-", dir=directory)
    source_sha1_hash = hashlib.sha1()
    verification_hash = hashlib.new(checksum_algorithm) if checksum_algorithm else None
    try:
        with os.fdopen(descriptor, "wb") as target:
            with urllib.request.urlopen(url, timeout=30) as response:
                while True:
                    chunk = response.read(1024 * 1024)
                    if not chunk:
                        break
                    target.write(chunk)
                    source_sha1_hash.update(chunk)
                    if verification_hash:
                        verification_hash.update(chunk)
            target.flush()
            os.fsync(target.fileno())
        source_sha1 = source_sha1_hash.hexdigest()
        if verification_hash and verification_hash.hexdigest().lower() != checksum_value.lower():
            raise RuntimeError("downloaded file checksum does not match the expected checksum")
        content_changed = not existing or digest_file(dest, "sha1") != source_sha1
        if content_changed:
            os.replace(temporary, dest)
            temporary = None
    finally:
        if temporary and os.path.exists(temporary):
            os.unlink(temporary)

metadata = os.stat(dest)
mode_changed = False
if p.get("mode") is not None:
    desired_mode = int(str(p["mode"]), 8)
    mode_changed = (metadata.st_mode & 0o7777) != desired_mode
    if mode_changed:
        os.chmod(dest, desired_mode)

uid = desired_identity(p.get("owner"), lambda value: pwd.getpwnam(value).pw_uid)
gid = desired_identity(p.get("group"), lambda value: grp.getgrnam(value).gr_gid)
owner_changed = (uid != -1 and metadata.st_uid != uid) or (gid != -1 and metadata.st_gid != gid)
if owner_changed:
    os.chown(dest, uid, gid)

print(json.dumps({
    "changed": content_changed or mode_changed or owner_changed,
    "checksum_src": source_sha1,
    "dest": dest,
}))
"#;

pub fn execute(
    connection: &dyn SshConnection,
    args: &Value,
    use_become: bool,
    become_user: &str,
    check_mode: bool,
) -> Result<ModuleResult> {
    validate_params(
        args,
        &["url", "dest", "owner", "group", "mode", "force", "checksum"],
    )?;
    let url = get_param::<String>(args, "url")?;
    let dest = get_param::<String>(args, "dest")?;
    if url.is_empty() || dest.is_empty() {
        bail!("Get_url url and dest cannot be empty");
    }
    let parameters = serde_json::json!({
        "url": url,
        "dest": dest,
        "owner": get_optional_param::<String>(args, "owner")?,
        "group": get_optional_param::<String>(args, "group")?,
        "mode": get_optional_param::<String>(args, "mode")?,
        "force": get_optional_param::<bool>(args, "force")?.unwrap_or(false),
        "checksum": get_optional_param::<String>(args, "checksum")?,
        "check_mode": check_mode,
    });
    let output = execute_json_mapping(
        connection,
        GET_URL_SCRIPT,
        &[serde_json::to_string(&parameters)?],
        use_become,
        become_user,
    )?;
    let changed = output
        .get(Value::String("changed".to_string()))
        .and_then(Value::as_bool)
        .context("Get_url module did not return changed")?;
    let values = output
        .into_iter()
        .filter_map(|(key, value)| match key {
            Value::String(key) if key != "changed" => Some((key, value)),
            _ => None,
        })
        .collect::<HashMap<_, _>>();

    Ok(ModuleResult {
        changed,
        msg: if check_mode {
            "URL is reachable; check mode did not download it".to_string()
        } else if changed {
            "File downloaded".to_string()
        } else {
            "Destination is already up to date".to_string()
        },
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
    use crate::ssh::connection::MockSshConnection;

    #[test]
    fn remote_download_result_exposes_ansible_checksum_fields() {
        let mut connection = MockSshConnection::new();
        connection
            .expect_execute_command_with_input()
            .once()
            .withf(|command, script| {
                command.starts_with("python3 - ")
                    && script.windows(14).any(|window| window == b"urllib.request")
            })
            .returning(|_, _| {
                Ok((
                    0,
                    r#"{"changed":true,"checksum_src":"aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d","dest":"/tmp/artifact"}"#.to_string(),
                    String::new(),
                ))
            });
        let args: Value = serde_yaml::from_str(
            "url: http://example.invalid/artifact\ndest: /tmp/artifact\nmode: '0755'\nforce: true\nchecksum: sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824\n",
        )
        .unwrap();

        let result = execute(&connection, &args, false, "root", false).unwrap();
        assert!(result.changed);
        assert_eq!(
            result.values.get("checksum_src"),
            Some(&Value::String(
                "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d".to_string()
            ))
        );
    }
}
