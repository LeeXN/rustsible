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
import shutil
import stat
import sys
import tempfile
import urllib.parse
import urllib.request

p = json.loads(sys.argv[1])
url, dest = p["url"], p["dest"]
if urllib.parse.urlparse(url).scheme.lower() not in ("http", "https"):
    raise RuntimeError("unsupported URL scheme: expected http or https")

def digest_file(path, algorithm):
    digest = hashlib.new(algorithm)
    with open(path, "rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()

def desired_identity(value, resolver):
    if value is None:
        return None
    if not value or ":" in value or any(ord(c) < 32 for c in value):
        raise RuntimeError("invalid file owner or group")
    return int(value) if value.isdigit() else resolver(value)

def snapshot():
    try:
        metadata = os.lstat(dest)
    except FileNotFoundError:
        return None
    if not stat.S_ISREG(metadata.st_mode):
        raise RuntimeError("destination must be a regular file, not a symbolic link or directory")
    return metadata

# Validate every requested setting before network traffic or file creation.
mode = p.get("mode")
if mode is not None:
    if not mode or any(c not in "01234567" for c in mode) or int(mode, 8) > 0o7777:
        raise RuntimeError("mode must be an octal string between 0000 and 7777")
    mode = int(mode, 8)
uid = desired_identity(p.get("owner"), lambda value: pwd.getpwnam(value).pw_uid)
gid = desired_identity(p.get("group"), lambda value: grp.getgrnam(value).gr_gid)
checksum = p.get("checksum")
checksum_algorithm = checksum_value = None
if checksum is not None:
    checksum_algorithm, separator, checksum_value = checksum.partition(":")
    try:
        size = hashlib.new(checksum_algorithm).digest_size
    except (ValueError, TypeError):
        raise RuntimeError("unsupported checksum algorithm")
    if not separator or size == 0 or len(checksum_value) != size * 2 or any(c not in "0123456789abcdefABCDEF" for c in checksum_value):
        raise RuntimeError("checksum must use <algorithm>:<hex digest> format")
    checksum_value = checksum_value.lower()

original = snapshot()
existing = original is not None
skip_download = existing and not p["force"] and (
    checksum_value is None or digest_file(dest, checksum_algorithm) == checksum_value
)
metadata_changed = existing and (
    (mode is not None and stat.S_IMODE(original.st_mode) != mode) or
    (uid is not None and original.st_uid != uid) or
    (gid is not None and original.st_gid != gid)
)
if skip_download and not metadata_changed:
    print(json.dumps({"changed": False, "checksum_src": digest_file(dest, "sha1"), "dest": dest}))
    raise SystemExit(0)
if p["check_mode"]:
    if not skip_download:
        with urllib.request.urlopen(urllib.request.Request(url, method="HEAD"), timeout=30):
            pass
    print(json.dumps({"changed": True, "dest": dest}))
    raise SystemExit(0)

directory = os.path.dirname(dest) or "."
# A private directory avoids exposing content when preserved permissions allow
# other users to read the prepared inode before its final atomic installation.
with tempfile.TemporaryDirectory(prefix=".rustsible-get-url-", dir=directory) as staging:
    temporary = os.path.join(staging, "content")
    source_sha1_hash = hashlib.sha1()
    verification_hash = hashlib.new(checksum_algorithm) if checksum_algorithm else None
    if skip_download:
        shutil.copyfile(dest, temporary)
        source_sha1 = digest_file(dest, "sha1")
    else:
        with open(temporary, "xb") as target:
            os.chmod(temporary, 0o600)
            with urllib.request.urlopen(url, timeout=30) as response:
                # HTTPResponse decrements length during reads; retain the
                # original framing length before streaming the response.
                expected_length = response.length
                received = 0
                while True:
                    chunk = response.read(1024 * 1024)
                    if not chunk:
                        break
                    target.write(chunk)
                    received += len(chunk)
                    source_sha1_hash.update(chunk)
                    if verification_hash:
                        verification_hash.update(chunk)
                if expected_length is not None and received != expected_length:
                    raise RuntimeError("incomplete HTTP response body")
            target.flush()
            os.fsync(target.fileno())
        source_sha1 = source_sha1_hash.hexdigest()
        if verification_hash and verification_hash.hexdigest().lower() != checksum_value:
            raise RuntimeError("downloaded file checksum does not match the expected checksum")
    content_changed = not existing or digest_file(dest, "sha1") != source_sha1
    if content_changed or metadata_changed:
        # Copy protection metadata after writing: writing/chown can clear file
        # capabilities, and silently dropping an xattr is never acceptable.
        os.chown(temporary,
            uid if uid is not None else (original.st_uid if existing else -1),
            gid if gid is not None else (original.st_gid if existing else -1))
        if existing:
            for name in os.listxattr(dest):
                os.setxattr(temporary, name, os.getxattr(dest, name))
        os.chmod(temporary, mode if mode is not None else (stat.S_IMODE(original.st_mode) if existing else 0o644))
        with open(temporary, "rb") as prepared:
            os.fsync(prepared.fileno())
        current = snapshot()
        def identity(value):
            return None if value is None else (value.st_dev, value.st_ino, value.st_mode, value.st_uid, value.st_gid, value.st_size, value.st_mtime_ns, value.st_ctime_ns)
        if identity(current) != identity(original):
            raise RuntimeError("destination changed during download")
        os.replace(temporary, dest)

print(json.dumps({"changed": content_changed or metadata_changed, "checksum_src": source_sha1, "dest": dest}))
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
    use crate::ssh::connection::LocalConnection;
    use crate::ssh::connection::MockSshConnection;
    use std::io::{Read, Write};
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    fn http_response(
        bytes: &'static [u8],
        expected_length: Option<usize>,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/artifact", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                            .unwrap();
                        let mut request = [0; 4096];
                        let count = stream.read(&mut request).unwrap();
                        assert!(count > 0);
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            expected_length.unwrap_or(bytes.len())
                        )
                        .unwrap();
                        stream.write_all(bytes).unwrap();
                        return;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "no HTTP request received"
                        );
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(error) => panic!("HTTP fixture failed: {error}"),
                }
            }
        });
        (url, server)
    }

    #[test]
    fn force_false_keeps_existing_file_without_contacting_server() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("artifact");
        std::fs::write(&path, b"original").unwrap();
        let connection = LocalConnection::new(&Host::new("localhost")).unwrap();
        let args = serde_yaml::to_value(serde_json::json!({
            "url": "http://127.0.0.1:1/unreachable", "dest": path, "force": false,
        }))
        .unwrap();
        for check in [false, true] {
            assert!(
                !execute(&connection, &args, false, "root", check)
                    .unwrap()
                    .changed
            );
        }
        assert_eq!(std::fs::read(path).unwrap(), b"original");
    }

    #[test]
    fn checksum_mismatch_updates_with_force_false_and_preserves_mode_and_identity() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("artifact");
        std::fs::write(&path, b"original").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let original = std::fs::metadata(&path).unwrap();
        crate::testing::test_xattr(&path, "user.rustsible-protection", Some(b"keep-protection"));
        let (url, server) = http_response(b"hello", None);
        let args = serde_yaml::to_value(serde_json::json!({
            "url": url, "dest": path, "force": false,
            "checksum": "sha1:aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d",
        }))
        .unwrap();
        let connection = LocalConnection::new(&Host::new("localhost")).unwrap();
        assert!(
            execute(&connection, &args, false, "root", false)
                .unwrap()
                .changed
        );
        server.join().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");
        assert_eq!(
            crate::testing::test_xattr(&path, "user.rustsible-protection", None),
            b"keep-protection"
        );
        let installed = std::fs::metadata(&path).unwrap();
        assert_eq!(
            (installed.mode(), installed.uid(), installed.gid()),
            (original.mode(), original.uid(), original.gid())
        );
        assert!(
            !execute(&connection, &args, false, "root", false)
                .unwrap()
                .changed
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_verification_and_invalid_metadata_preserve_original_inode() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("artifact");
        std::fs::write(&path, b"original").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o751)).unwrap();
        let original = std::fs::metadata(&path).unwrap();
        let connection = LocalConnection::new(&Host::new("localhost")).unwrap();
        let (url, server) = http_response(b"corrupt", None);
        let args = serde_yaml::to_value(serde_json::json!({
            "url": url, "dest": path, "force": true,
            "checksum": "sha1:aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d",
        }))
        .unwrap();
        assert!(execute(&connection, &args, false, "root", false)
            .unwrap_err()
            .to_string()
            .contains("checksum does not match"));
        server.join().unwrap();
        for (key, value, error) in [
            ("mode", "888", "mode must"),
            ("owner", "missing-rustsible-account-99999", "getpwnam"),
            ("checksum", "sha1:not-a-digest", "checksum must"),
        ] {
            let mut args = serde_json::json!({"url": "http://127.0.0.1:1/unreachable", "dest": path, "force": true});
            args[key] = value.into();
            let args = serde_yaml::to_value(args).unwrap();
            for check in [false, true] {
                assert!(execute(&connection, &args, false, "root", check)
                    .unwrap_err()
                    .to_string()
                    .contains(error));
            }
        }
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        let after = std::fs::metadata(&path).unwrap();
        assert_eq!(
            (after.ino(), after.mode(), after.uid(), after.gid()),
            (
                original.ino(),
                original.mode(),
                original.uid(),
                original.gid()
            )
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn truncated_http_body_does_not_replace_existing_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("artifact");
        std::fs::write(&path, b"original").unwrap();
        let original = std::fs::metadata(&path).unwrap();
        let (url, server) = http_response(b"short", Some(999));
        let args =
            serde_yaml::to_value(serde_json::json!({"url": url, "dest": path, "force": true}))
                .unwrap();
        let connection = LocalConnection::new(&Host::new("localhost")).unwrap();
        assert!(execute(&connection, &args, false, "root", false).is_err());
        server.join().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        let after = std::fs::metadata(&path).unwrap();
        assert_eq!(
            (after.ino(), after.mode(), after.uid(), after.gid()),
            (
                original.ino(),
                original.mode(),
                original.uid(),
                original.gid()
            )
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

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
