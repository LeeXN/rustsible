//! Diff data is bounded independently from module output. Binary/large files
//! carry digests and sizes; failed reads never turn an otherwise valid metadata
//! repair into a failure. no_log is enforced by the shared result presenters.
use rustsible_protocol::{ModuleResult, Request};
use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Seek},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};
const MAX_TEXT: u64 = 64 * 1024;
pub fn before(request: &Request) -> Option<Value> {
    if !request.diff || request.extension.is_some() {
        return None;
    }
    path(request).map(snapshot)
}
fn path(request: &Request) -> Option<&str> {
    request
        .args
        .get("dest")
        .or_else(|| request.args.get("path"))
        .and_then(serde_yaml::Value::as_str)
}
pub fn finish(
    request: &Request,
    payload: Option<&Path>,
    before: Option<Value>,
    result: &mut ModuleResult,
) {
    if !request.diff || !result.changed || request.extension.is_some() {
        return;
    }
    let diff = if request.module == "git" {
        json!({"before":result.values.get("before"),"after":result.values.get("after")})
    } else if let (Some(before), Some(path)) = (before, path(request)) {
        let after = if request.check {
            predicted(request, payload, &before)
        } else {
            snapshot(path)
        };
        json!({"before":before,"after":after,"entries":result.values.get("entries")})
    } else {
        return;
    };
    if let Ok(value) = serde_yaml::to_value(diff) {
        result.values.insert("diff".into(), value);
    }
}
fn snapshot(path: &str) -> Value {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return json!({"path":path,"exists":false})
        }
        Err(_) => return json!({"path":path,"content_unavailable":true}),
    };
    let mut value = json!({"path":path,"exists":true,"mode":format!("{:04o}",metadata.mode() & 0o7777),"uid":metadata.uid(),"gid":metadata.gid(),"size":metadata.len(),"type":if metadata.is_dir(){"directory"}else if metadata.is_file(){"file"}else if metadata.file_type().is_symlink(){"link"}else{"special"}});
    if metadata.file_type().is_symlink() {
        value["target"] = fs::read_link(path)
            .ok()
            .map(|path| path.to_string_lossy().into_owned())
            .into();
    }
    if metadata.is_file() {
        let content = (|| -> anyhow::Result<()> {
            let mut file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(path)?;
            let opened = file.metadata()?;
            if !opened.is_file() || opened.ino() != metadata.ino() || opened.dev() != metadata.dev()
            {
                anyhow::bail!("Diff file changed");
            }
            value["sha256"] = crate::modules::hash(&mut file, "sha256")?.into();
            file.rewind()?;
            let mut bytes = Vec::new();
            file.take(MAX_TEXT + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 <= MAX_TEXT && !bytes.contains(&0) {
                if let Ok(text) = String::from_utf8(bytes) {
                    value["content"] = text.into();
                } else {
                    value["binary"] = true.into();
                }
            } else if metadata.len() > MAX_TEXT {
                value["content_omitted"] = "text or binary content exceeds 64 KiB".into();
            } else {
                value["binary"] = true.into();
            }
            Ok(())
        })();
        if content.is_err() {
            value["content_unavailable"] = true.into();
        }
    }
    value
}
fn predicted(request: &Request, payload: Option<&Path>, before: &Value) -> Value {
    let mut after = before.clone();
    after["predicted"] = true.into();
    match request.module.as_str() {
        "copy" | "template"
            if !(request
                .args
                .get("force")
                .and_then(serde_yaml::Value::as_bool)
                == Some(false)
                && before["exists"] == true) =>
        {
            if let Some(payload) = payload.and_then(Path::to_str) {
                after = snapshot(payload);
                after["path"] = before["path"].clone();
                after["predicted"] = true.into();
                after["mode"] = if before["exists"] == true {
                    before["mode"].clone()
                } else {
                    format!("{:04o}", 0o666 & !crate::filesystem::creation_umask()).into()
                };
                after["uid"] = before
                    .get("uid")
                    .cloned()
                    .unwrap_or_else(|| json!(unsafe { libc::geteuid() }));
                after["gid"] = before.get("gid").cloned().unwrap_or_else(|| {
                    let inherited = before["path"]
                        .as_str()
                        .and_then(|path| Path::new(path).parent())
                        .and_then(|path| fs::metadata(path).ok())
                        .filter(|metadata| metadata.mode() & 0o2000 != 0)
                        .map(|metadata| metadata.gid())
                        .unwrap_or_else(|| unsafe { libc::getegid() });
                    json!(inherited)
                });
            }
        }
        "lineinfile" => {
            if let Some(content) = before["content"]
                .as_str()
                .or_else(|| (before["exists"] == false).then_some(""))
            {
                let mut text = content.to_owned();
                let string = |key| {
                    request
                        .args
                        .get(key)
                        .and_then(serde_yaml::Value::as_str)
                        .map(str::to_owned)
                };
                if crate::lines::process_line_modifications(
                    &mut text,
                    string("line"),
                    string("regexp"),
                    string("state").as_deref().unwrap_or("present"),
                    string("insertafter"),
                    string("insertbefore"),
                )
                .is_ok()
                    && text.len() as u64 <= MAX_TEXT
                {
                    after["size"] = json!(text.len());
                    after["exists"] = true.into();
                    after["content"] = text.into();
                    after.as_object_mut().unwrap().remove("sha256");
                }
            }
        }
        "file" => {
            let state = request
                .args
                .get("state")
                .and_then(serde_yaml::Value::as_str)
                .unwrap_or("file");
            if state.eq_ignore_ascii_case("absent") {
                return json!({"path":before["path"],"exists":false,"predicted":true});
            }
            after["state"] = state.into();
            after["exists"] = true.into();
            if state == "link" {
                after["target"] = request
                    .args
                    .get("src")
                    .and_then(serde_yaml::Value::as_str)
                    .into();
            }
        }
        _ => {
            for key in ["content", "sha256", "binary", "size"] {
                after.as_object_mut().unwrap().remove(key);
            }
            if request.module == "get_url" {
                after["exists"] = true.into();
            }
            after["content_omitted"] =
                "Predicted change; content is available only after execution".into();
        }
    }
    if let Ok(attributes) = crate::filesystem::Attributes::parse(&request.args) {
        if let Some(mode) = attributes.mode {
            after["mode"] = format!("{mode:04o}").into();
        }
        if let Some(uid) = attributes.uid {
            after["uid"] = uid.into();
        }
        if let Some(gid) = attributes.gid {
            after["gid"] = gid.into();
        }
    }
    after
}
