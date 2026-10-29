use anyhow::{bail, Context, Result};
use rustsible_protocol::{ModuleResult, Request};
use serde_json::json;
use sha2::Digest;
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
};

pub fn execute(
    request: &Request,
    scratch: &std::path::Path,
    payload: Option<&std::path::Path>,
) -> ModuleResult {
    let _changes = rustsible_protocol::change::ChangeScope::enter(request.check);
    match run(request, scratch, payload) {
        Ok(mut result) => {
            result.changed |= rustsible_protocol::change::known_changed();
            result
        }
        Err(error) => {
            let mut result = error
                .downcast_ref::<crate::tools::Failure>()
                .map(|failure| failure.0.clone())
                .unwrap_or_else(|| ModuleResult {
                    failed: true,
                    ..ModuleResult::default()
                });
            result.changed |= rustsible_protocol::change::known_changed();
            result.msg = format!("{error:#}");
            result
        }
    }
}
fn run(
    request: &Request,
    scratch: &std::path::Path,
    payload: Option<&std::path::Path>,
) -> Result<ModuleResult> {
    let args = request
        .args
        .as_mapping()
        .context("Native module arguments must be a mapping")?;
    if matches!(request.module.as_str(), "command" | "shell") {
        if args.len() != 1 {
            bail!("Command modules accept only cmd");
        }
        let command = request
            .args
            .get("cmd")
            .and_then(serde_yaml::Value::as_str)
            .filter(|value| !value.trim().is_empty() && !value.contains('\0'))
            .context("Command requires non-empty cmd without NUL bytes")?;
        if request.module == "command" {
            rustsible_protocol::tokenize_args(command)?;
        }
        if request.check {
            return Ok(ModuleResult {
                skipped: true,
                msg: "Check mode: command was not executed".into(),
                ..ModuleResult::default()
            });
        }
        return crate::process::execute(command, request.module == "shell", scratch);
    }
    match request.module.as_str() {
        "service" | "systemd" => {
            return crate::services::execute(
                &request.args,
                request.check,
                request.module == "systemd",
            )
        }
        "package" => return crate::packages::execute(&request.args, request.check),
        "user" => return crate::accounts::execute(&request.args, request.check),
        "copy" | "template" => {
            return crate::files::copy(
                &request.args,
                payload.context("Missing streamed content")?,
                request.check,
            )
        }
        "get_url" => return crate::http::get_url(request),
        "file" => return crate::files::file(&request.args, request.check),
        "lineinfile" => return crate::files::lineinfile(&request.args, request.check),
        _ => {}
    }
    let allowed = match request.module.as_str() {
        "setup" => &[][..],
        "stat" => &["path", "checksum_algorithm"][..],
        _ => bail!("Unsupported native module"),
    };
    for key in args.keys() {
        if !key.as_str().is_some_and(|key| allowed.contains(&key)) {
            bail!("Unsupported native module parameter");
        }
    }
    let mut result = ModuleResult::default();
    match request.module.as_str() {
        "setup" => {
            let now = chrono::Local::now();
            result.values.insert("ansible_facts".into(), serde_yaml::to_value(json!({
                "architecture": std::env::consts::ARCH, "system": "Linux",
                "date_time": { "date": now.format("%Y-%m-%d").to_string(), "time": now.format("%H:%M:%S").to_string(),
                    "year": now.format("%Y").to_string(), "month": now.format("%m").to_string(), "day": now.format("%d").to_string(),
                    "hour": now.format("%H").to_string(), "minute": now.format("%M").to_string(), "second": now.format("%S").to_string(),
                    "weekday": now.format("%A").to_string(), "weekday_number": now.format("%w").to_string(),
                    "epoch": now.timestamp().to_string(), "iso8601": now.with_timezone(&chrono::Utc).format("%Y-%m-%dT%H:%M:%SZ").to_string() }
            }))?);
            result.msg = "Facts gathered".into();
        }
        "stat" => {
            let path = request
                .args
                .get("path")
                .and_then(serde_yaml::Value::as_str)
                .filter(|value| !value.is_empty() && !value.contains('\0'))
                .context("Stat requires a non-empty path")?;
            let algorithm = match request.args.get("checksum_algorithm") {
                Some(serde_yaml::Value::String(value)) => value.as_str(),
                None => "sha1",
                _ => bail!("checksum_algorithm must be a string"),
            };
            rustsible_protocol::digest_hex_length(algorithm)?;
            let value = match fs::symlink_metadata(path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    json!({"exists": false})
                }
                Err(error) => return Err(error).context("Cannot inspect stat path"),
                Ok(metadata) => {
                    let mut value = json!({"exists": true, "isreg": metadata.is_file(), "isdir": metadata.is_dir(), "islnk": metadata.file_type().is_symlink(),
                        "mode": format!("{:04o}", metadata.mode() & 0o7777), "uid": metadata.uid(), "gid": metadata.gid(), "size": metadata.len(), "inode": metadata.ino(), "dev": metadata.dev(), "nlink": metadata.nlink(),
                        "mtime": metadata.mtime() as f64 + metadata.mtime_nsec() as f64 / 1e9, "ctime": metadata.ctime() as f64 + metadata.ctime_nsec() as f64 / 1e9 });
                    if metadata.is_file() {
                        let mut file = OpenOptions::new()
                            .read(true)
                            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                            .open(path)
                            .context("Cannot open stat path without following links")?;
                        let opened = file.metadata()?;
                        if !opened.is_file()
                            || opened.ino() != metadata.ino()
                            || opened.dev() != metadata.dev()
                        {
                            bail!("Stat path changed during inspection");
                        }
                        value["checksum"] = hash(&mut file, algorithm)?.into();
                        let after = file.metadata()?;
                        if (
                            opened.len(),
                            opened.mtime(),
                            opened.mtime_nsec(),
                            opened.ctime(),
                            opened.ctime_nsec(),
                        ) != (
                            after.len(),
                            after.mtime(),
                            after.mtime_nsec(),
                            after.ctime(),
                            after.ctime_nsec(),
                        ) {
                            bail!("Stat file changed while calculating checksum");
                        }
                    }
                    value
                }
            };
            result
                .values
                .insert("stat".into(), serde_yaml::to_value(value)?);
            result.msg = "Path inspected".into();
        }
        _ => unreachable!(),
    }
    Ok(result)
}
pub(crate) fn hash(file: &mut File, algorithm: &str) -> Result<String> {
    fn calculate<D: Digest>(file: &mut File) -> Result<String> {
        let mut digest = D::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            let length = file.read(&mut buffer)?;
            if length == 0 {
                break;
            }
            digest.update(&buffer[..length]);
        }
        Ok(digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }
    match algorithm {
        "md5" => calculate::<md5::Md5>(file),
        "sha1" => calculate::<sha1::Sha1>(file),
        "sha224" => calculate::<sha2::Sha224>(file),
        "sha256" => calculate::<sha2::Sha256>(file),
        "sha384" => calculate::<sha2::Sha384>(file),
        "sha512" => calculate::<sha2::Sha512>(file),
        _ => bail!("Unsupported checksum algorithm"),
    }
}
