//! Versioned, bounded stdin/stdout messages shared by controller, workers and
//! native executable extensions. No transport or playbook implementation lives here.
mod args;
pub mod change;
pub mod param;
pub mod registry;
mod result;
use anyhow::{bail, Context, Result};
pub use args::{tokenize_args, CommandArgs};
pub use result::{insert_output_values, result_fields, ModuleResult, Outcome};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

pub const VERSION: u32 = 5;
pub const BUILD_ID: &str = env!("RUSTSIBLE_BUILD_ID");
pub const MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;
// JSON escaping can expand a bounded 16 MiB command response sixfold.
pub const MAX_RESPONSE_BYTES: usize = 128 * 1024 * 1024;
pub const NATIVE_MODULES: &[&str] = &[
    "setup",
    "stat",
    "command",
    "shell",
    "copy",
    "template",
    "file",
    "lineinfile",
    "get_url",
    "user",
    "service",
    "systemd",
    "package",
    "async_status",
    "group",
    "wait_for",
    "uri",
    "git",
    "unarchive",
];

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub build_id: String,
    pub module: String,
    pub args: serde_yaml::Value,
    pub check: bool,
    pub diff: bool,
    pub timeout_ms: u64,
    pub environment: std::collections::BTreeMap<String, String>,
    pub managed: bool,
    pub payload: Option<Payload>,
    pub asynchronous: Option<AsyncSpec>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AsyncSpec {
    pub jid: String,
    pub timeout_ms: u64,
    pub directory: Option<String>,
    pub no_log: bool,
}
impl AsyncSpec {
    pub fn validate(&self) -> Result<()> {
        validate_job_id(&self.jid)?;
        if self.timeout_ms == 0 || self.timeout_ms > 7 * 24 * 60 * 60 * 1000 {
            bail!("Async timeout must be between one millisecond and seven days");
        }
        if let Some(directory) = &self.directory {
            validate_async_directory(directory)?;
        }
        Ok(())
    }
}
pub fn validate_job_id(jid: &str) -> Result<()> {
    if jid.len() != 65
        || !jid.starts_with('j')
        || !jid[1..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("Invalid native async job ID");
    }
    Ok(())
}
pub fn validate_async_directory(path: &str) -> Result<()> {
    if (!std::path::Path::new(path).is_absolute() && !path.starts_with("~/"))
        || path.chars().any(char::is_control)
    {
        bail!(
            "Async directory must be absolute or begin with ~/ and contain no control characters"
        );
    }
    Ok(())
}
impl Request {
    pub fn new(module: &str, args: serde_yaml::Value, check: bool, timeout_ms: u64) -> Self {
        Self {
            version: VERSION,
            build_id: BUILD_ID.into(),
            module: module.into(),
            args,
            check,
            diff: false,
            timeout_ms,
            environment: Default::default(),
            managed: true,
            payload: None,
            asynchronous: None,
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != VERSION {
            bail!("Worker protocol version mismatch");
        }
        if self.build_id != BUILD_ID {
            bail!("Worker build identifier mismatch");
        }
        if self.timeout_ms == 0 || self.timeout_ms > 7 * 24 * 60 * 60 * 1000 {
            bail!("Invalid worker timeout");
        }
        for (name, value) in &self.environment {
            if name.is_empty()
                || name.bytes().enumerate().any(|(index, byte)| {
                    !(byte.is_ascii_alphabetic()
                        || byte == b'_'
                        || (index > 0 && byte.is_ascii_digit()))
                })
                || value.contains('\0')
            {
                bail!("Invalid environment name or NUL byte in value");
            }
        }
        if !NATIVE_MODULES.contains(&self.module.as_str()) {
            bail!("Unsupported native module: {}", self.module);
        }
        registry::Builtin::lookup(&self.module)?.validate_capabilities(
            self.check,
            self.diff,
            self.asynchronous.is_some(),
        )?;
        if let Some(asynchronous) = &self.asynchronous {
            asynchronous.validate()?;
            if self.check {
                bail!("Async tasks cannot start in check mode");
            }
            registry::Builtin::lookup(&self.module)?.validate_capabilities(false, false, true)?;
            if self.payload.is_some() {
                bail!("Async tasks cannot receive controller file payloads");
            }
        }
        if (matches!(self.module.as_str(), "copy" | "template")
            || self.module == "unarchive"
                && self
                    .args
                    .get("remote_src")
                    .and_then(serde_yaml::Value::as_bool)
                    != Some(true))
            != self.payload.is_some()
        {
            bail!("Streamed payload is required only by copy/template and controller-source unarchive");
        }
        if let Some(payload) = &self.payload {
            if payload.sha256.len() != 64
                || !payload.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                bail!("Invalid payload SHA-256");
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub size: u64,
    pub sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadReady {
    pub version: u32,
    pub build_id: String,
}
impl PayloadReady {
    pub fn new() -> Self {
        Self {
            version: VERSION,
            build_id: BUILD_ID.into(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != VERSION || self.build_id != BUILD_ID {
            bail!("Payload handshake version/build mismatch");
        }
        Ok(())
    }
}
impl Default for PayloadReady {
    fn default() -> Self {
        Self::new()
    }
}

/// Input staging can fail before the target is ready to accept a body. Such a
/// module failure must not be misreported as an unreachable SSH host.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum PayloadAck {
    Ready(PayloadReady),
    Result(Response),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    pub version: u32,
    pub build_id: String,
    pub os: String,
    pub arch: String,
    pub uid: u32,
    pub modules: Vec<String>,
}
impl Hello {
    pub fn validate(&self, arch: &str, uid: u32, module: &str) -> Result<()> {
        if self.version != VERSION {
            bail!("Worker protocol version mismatch");
        }
        if self.build_id != BUILD_ID {
            bail!("Worker build identifier mismatch");
        }
        if self.os != "linux" || self.arch != arch {
            bail!("Worker operating system/architecture mismatch");
        }
        if self.uid != uid {
            bail!("Worker execution identity mismatch");
        }
        if !self.modules.iter().any(|name| name == module) {
            bail!("Worker does not support requested module");
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub version: u32,
    pub build_id: String,
    pub result: ModuleResult,
}
impl Response {
    pub fn new(result: ModuleResult) -> Self {
        Self {
            version: VERSION,
            build_id: BUILD_ID.into(),
            result,
        }
    }
    pub fn validate(self) -> Result<ModuleResult> {
        if self.version != VERSION || self.build_id != BUILD_ID {
            bail!("Worker response version/build mismatch");
        }
        Ok(self.result)
    }
}

/// Length framing separates a complete request from EOF. The writer keeps
/// stdin open after the frame so EOF means cancellation, not request completion.
pub fn read_frame<T: DeserializeOwned>(reader: &mut impl Read, limit: usize) -> Result<T> {
    let mut prefix = [0; 4];
    reader
        .read_exact(&mut prefix)
        .context("Incomplete protocol frame header")?;
    let length = u32::from_be_bytes(prefix) as usize;
    if length == 0 || length > limit {
        bail!("Protocol frame exceeds its size bound");
    }
    let mut body = vec![0; length];
    reader
        .read_exact(&mut body)
        .context("Incomplete protocol frame body")?;
    serde_json::from_slice(&body).context("Invalid protocol frame")
}
pub fn write_frame<T: Serialize>(writer: &mut impl Write, value: &T, limit: usize) -> Result<()> {
    let body = serde_json::to_vec(value).context("Cannot encode protocol frame")?;
    if body.is_empty() || body.len() > limit || body.len() > u32::MAX as usize {
        bail!("Protocol frame exceeds its size bound");
    }
    writer.write_all(&(body.len() as u32).to_be_bytes())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}
pub fn encode<T: Serialize>(value: &T, limit: usize) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    write_frame(&mut output, value, limit)?;
    Ok(output)
}
pub fn decode<T: DeserializeOwned>(bytes: &[u8], limit: usize) -> Result<T> {
    let mut reader = bytes;
    let value = read_frame(&mut reader, limit)?;
    if !reader.is_empty() {
        bail!("Trailing bytes after protocol response");
    }
    Ok(value)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub arch: String,
    pub file: String,
    pub sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub build_id: String,
    pub workers: Vec<Artifact>,
}
impl Manifest {
    pub fn validate(&self) -> Result<()> {
        if self.version != VERSION || self.build_id != BUILD_ID {
            bail!("Worker manifest version/build mismatch");
        }
        let mut architectures = std::collections::HashSet::new();
        for worker in &self.workers {
            if !matches!(worker.arch.as_str(), "x86_64" | "aarch64")
                || !architectures.insert(&worker.arch)
            {
                bail!("Unsupported or duplicate worker architecture");
            }
            if worker.file.is_empty()
                || worker.file.contains("..")
                || !worker
                    .file
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
            {
                bail!("Invalid worker artifact filename");
            }
            if worker.sha256.len() != 64
                || !worker.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                bail!("Invalid worker artifact SHA-256");
            }
        }
        if self.workers.is_empty() {
            bail!("Worker manifest has no artifacts");
        }
        Ok(())
    }
}
pub fn sha256(reader: &mut impl Read) -> Result<String> {
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let length = reader.read(&mut buffer)?;
        if length == 0 {
            break;
        }
        digest.update(&buffer[..length]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
pub fn digest_hex_length(algorithm: &str) -> Result<usize> {
    match algorithm {
        "md5" => Ok(32),
        "sha1" => Ok(40),
        "sha224" => Ok(56),
        "sha256" => Ok(64),
        "sha384" => Ok(96),
        "sha512" => Ok(128),
        _ => bail!("Unsupported checksum algorithm: {algorithm}"),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn async_requests_validate_identity_capability_directory_and_check_before_execution() {
        let specification = super::AsyncSpec {
            jid: format!("j{:064x}", 1),
            timeout_ms: 1000,
            directory: Some("~/private-jobs".into()),
            no_log: false,
        };
        assert!(specification.validate().is_ok());
        for jid in ["../state", "", "jbad", "j000/../../state"] {
            assert!(super::validate_job_id(jid).is_err());
        }
        for directory in ["relative", "", "/tmp/invalid\0path"] {
            assert!(super::validate_async_directory(directory).is_err());
        }
        let mut request = super::Request::new(
            "command",
            serde_yaml::from_str("cmd: /bin/true").unwrap(),
            false,
            1000,
        );
        request.asynchronous = Some(specification);
        assert!(request.validate().is_ok());
        request.check = true;
        assert!(request.validate().is_err());
        request.check = false;
        request.module = "copy".into();
        assert!(request.validate().is_err());
    }
    use super::*;
    #[test]
    fn framing_rejects_truncation_oversize_trailing_and_unknown_fields() {
        let request = Request::new("stat", serde_yaml::Value::Null, true, 1000);
        let encoded = encode(&request, MAX_REQUEST_BYTES).unwrap();
        assert_eq!(
            decode::<Request>(&encoded, MAX_REQUEST_BYTES)
                .unwrap()
                .module,
            "stat"
        );
        for length in [0, 1, 3, encoded.len() - 1] {
            assert!(decode::<Request>(&encoded[..length], MAX_REQUEST_BYTES).is_err());
        }
        assert!(decode::<Request>(&[255; 4], MAX_REQUEST_BYTES).is_err());
        let mut extra = encoded.clone();
        extra.push(0);
        assert!(decode::<Request>(&extra, MAX_REQUEST_BYTES).is_err());
        let mut value = serde_json::to_value(&request).unwrap();
        value["unexpected"] = true.into();
        assert!(decode::<Request>(
            &encode(&value, MAX_REQUEST_BYTES).unwrap(),
            MAX_REQUEST_BYTES
        )
        .is_err());
    }
    #[test]
    fn environment_and_execution_timeout_are_validated_before_dispatch() {
        let mut request = Request::new("stat", serde_yaml::Value::Null, true, 1000);
        request
            .environment
            .insert("VALID_1".into(), "literal $(value)".into());
        assert!(request.validate().is_ok());
        for (name, value) in [
            ("1BAD", "x"),
            ("BAD=NAME", "x"),
            ("", "x"),
            ("VALID", "nul\0value"),
        ] {
            request.environment = std::collections::BTreeMap::from([(name.into(), value.into())]);
            assert!(request.validate().is_err());
        }
        request.environment.clear();
        request.timeout_ms = 0;
        assert!(request.validate().is_err());
        request.timeout_ms = 7 * 24 * 60 * 60 * 1000 + 1;
        assert!(request.validate().is_err());
    }
    #[test]
    fn identity_and_version_handshakes_are_strict() {
        let mut hello = Hello {
            version: VERSION,
            build_id: BUILD_ID.into(),
            os: "linux".into(),
            arch: "x86_64".into(),
            uid: 1000,
            modules: vec!["stat".into()],
        };
        assert!(hello.validate("x86_64", 1000, "stat").is_ok());
        assert!(hello.validate("aarch64", 1000, "stat").is_err());
        assert!(hello.validate("x86_64", 0, "stat").is_err());
        assert!(hello.validate("x86_64", 1000, "copy").is_err());
        hello.version += 1;
        assert!(hello.validate("x86_64", 1000, "stat").is_err());
    }
    #[test]
    fn digest_is_streamed_and_manifest_paths_are_bounded() {
        assert_eq!(
            sha256(&mut &b"hello"[..]).unwrap(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        let mut manifest = Manifest {
            version: VERSION,
            build_id: BUILD_ID.into(),
            workers: vec![Artifact {
                arch: "x86_64".into(),
                file: "worker-x86_64".into(),
                sha256: "0".repeat(64),
            }],
        };
        assert!(manifest.validate().is_ok());
        manifest.workers[0].file = "../worker".into();
        assert!(manifest.validate().is_err());
    }
}
