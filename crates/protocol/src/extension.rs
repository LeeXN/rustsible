//! Stable executable extension ABI. Requests and results use bounded frames on
//! stdin/stdout; the handshake precedes parameter delivery. Extension builds
//! have their own identity rather than sharing the controller's Git build ID.
use crate::{ModuleResult, MAX_REQUEST_BYTES};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use std::{collections::BTreeMap, io};
pub const API_VERSION: u32 = 1;
pub const MAX_RESULT_BYTES: usize = 16 * 1024 * 1024;
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Module,
    Inventory,
    Lookup,
    Filter,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    pub check: bool,
    pub diff: bool,
    #[serde(rename = "async")]
    pub asynchronous: bool,
    pub mutates: bool,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParameterType {
    Any,
    String,
    Integer,
    Boolean,
    List,
    Mapping,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    #[serde(rename = "type")]
    pub kind: ParameterType,
    #[serde(default)]
    pub required: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub name: String,
    pub kind: Kind,
    pub capabilities: Capabilities,
    pub parameters: BTreeMap<String, Parameter>,
}
impl Definition {
    pub fn validate(&self) -> Result<()> {
        validate_name(&self.name)?;
        if self.kind != Kind::Module
            && (self.capabilities.mutates
                || !self.capabilities.check
                || self.capabilities.asynchronous
                || self.capabilities.diff)
        {
            bail!("Inventory/lookup/filter extensions must declare read-only check support and no diff/async");
        }
        for key in self.parameters.keys() {
            crate::param::validate_variable_name(key, "Extension parameter")?;
        }
        Ok(())
    }
    pub fn validate_call(&self, call: &Call) -> Result<()> {
        self.validate()?;
        if call.api_version != API_VERSION || call.name != self.name || call.kind != self.kind {
            bail!("Extension request identity or API version mismatch");
        }
        if call.check && !self.capabilities.check || call.diff && !self.capabilities.diff {
            bail!("Extension does not support the requested check/diff mode");
        }
        if call.timeout_ms == 0 || call.timeout_ms > 604800000 {
            bail!("Invalid extension timeout");
        }
        let args = call
            .args
            .as_mapping()
            .context("Extension arguments must be a mapping")?;
        for (name, parameter) in &self.parameters {
            if parameter.required && !args.contains_key(Value::String(name.clone())) {
                bail!("Missing required extension parameter: {name}");
            }
        }
        for (key, value) in args {
            let name = key
                .as_str()
                .context("Extension parameter names must be strings")?;
            let parameter = self
                .parameters
                .get(name)
                .with_context(|| format!("Unknown extension parameter: {name}"))?;
            let valid = match parameter.kind {
                ParameterType::Any => true,
                ParameterType::String => value.is_string(),
                ParameterType::Integer => value.as_i64().is_some() || value.as_u64().is_some(),
                ParameterType::Boolean => value.is_bool(),
                ParameterType::List => value.is_sequence(),
                ParameterType::Mapping => value.is_mapping(),
            };
            if !valid {
                bail!("Invalid type for extension parameter: {name}");
            }
        }
        Ok(())
    }
}
pub fn validate_name(name: &str) -> Result<()> {
    if name.len() > 255 {
        bail!("Extension name is too long");
    }
    let parts = name.split('.').collect::<Vec<_>>();
    if parts.len() != 3
        || name.len() > 255
        || parts.iter().any(|part| {
            part.is_empty()
                || !part.as_bytes()[0].is_ascii_lowercase()
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        })
    {
        bail!("Extension names must be namespace.collection.component");
    }
    if parts[0] == "ansible" && matches!(parts[1], "builtin" | "legacy") {
        bail!("Extensions cannot replace reserved builtin namespaces");
    }
    Ok(())
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Probe {
    pub api_version: u32,
    pub name: String,
    pub kind: Kind,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    pub api_version: u32,
    pub wire_version: u32,
    pub definition: Definition,
}
impl Hello {
    pub fn validate(&self, expected: &Definition) -> Result<()> {
        if self.api_version != API_VERSION
            || self.wire_version != crate::VERSION
            || &self.definition != expected
        {
            bail!("Native extension version, schema or capabilities mismatch");
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub api_version: u32,
    pub name: String,
    pub kind: Kind,
    pub args: Value,
    pub check: bool,
    pub diff: bool,
    pub timeout_ms: u64,
    #[serde(default)]
    pub no_log: bool,
}
impl Call {
    pub fn new(
        definition: &Definition,
        args: Value,
        check: bool,
        diff: bool,
        timeout_ms: u64,
    ) -> Self {
        Self {
            api_version: API_VERSION,
            name: definition.name.clone(),
            kind: definition.kind,
            args,
            check,
            diff,
            timeout_ms,
            no_log: false,
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    pub api_version: u32,
    pub result: ModuleResult,
}
impl Reply {
    pub fn validate(self) -> Result<ModuleResult> {
        if self.api_version != API_VERSION {
            bail!("Extension response API version mismatch");
        }
        Ok(self.result)
    }
}

/// SDK entry point for a native executable. The callback receives validated
/// arguments only after its advertised schema/capabilities have been sent.
pub fn serve(
    definitions: &[Definition],
    handler: impl FnOnce(&Definition, &Call) -> Result<ModuleResult>,
) -> Result<()> {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let probe: Probe = crate::read_frame(&mut input, MAX_REQUEST_BYTES)?;
    if probe.api_version != API_VERSION {
        bail!("Unsupported native extension API version");
    }
    let definition = definitions
        .iter()
        .find(|definition| definition.name == probe.name && definition.kind == probe.kind)
        .context("Extension component is not provided by this executable")?;
    definition.validate()?;
    crate::write_frame(
        &mut output,
        &Hello {
            api_version: API_VERSION,
            wire_version: crate::VERSION,
            definition: definition.clone(),
        },
        MAX_RESULT_BYTES,
    )?;
    let call: Call = crate::read_frame(&mut input, MAX_REQUEST_BYTES)?;
    definition.validate_call(&call)?;
    let _changes = crate::change::ChangeScope::enter(call.check);
    let mut result = handler(definition, &call).unwrap_or_else(|_| ModuleResult {
        failed: true,
        msg: "Native extension callback failed".into(),
        ..Default::default()
    });
    result.no_log |= call.no_log;
    crate::write_frame(
        &mut output,
        &Reply {
            api_version: API_VERSION,
            result,
        },
        MAX_RESULT_BYTES,
    )
}

/// Validate a pinned artifact before any process launch. This is also the
/// worker bootstrap check: foreign ELF images cannot trigger a shell fallback.
pub fn verify_binary(
    file: &mut std::fs::File,
    sha256: &str,
    arch: &str,
    label: &str,
) -> Result<()> {
    use std::io::{Read, Seek};
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 256 * 1024 * 1024 {
        bail!("{label} must be a regular executable no larger than 256 MiB");
    }
    if metadata.mode() & 0o6000 != 0 {
        bail!("{label} cannot carry setuid/setgid permissions");
    }
    file.rewind()?;
    if crate::sha256(file)? != sha256.to_ascii_lowercase() {
        bail!("{label} SHA-256 mismatch");
    }
    file.rewind()?;
    let mut header = [0u8; 64];
    file.read_exact(&mut header).with_context(|| {
        format!("Incomplete {label} ELF header; Python/scripts are unsupported")
    })?;
    let machine = match arch {
        "x86_64" => 62u16,
        "aarch64" => 183u16,
        _ => bail!("Unsupported native executable architecture"),
    };
    if &header[..4] != b"\x7fELF"
        || header[4..7] != [2, 1, 1]
        || !matches!(u16::from_le_bytes([header[16], header[17]]), 2 | 3)
        || u16::from_le_bytes([header[18], header[19]]) != machine
    {
        bail!("{label} is not a Linux ELF64 executable for target architecture {arch}; Python/scripts are unsupported");
    }
    file.rewind()?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub path: String,
    pub sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub definition: Definition,
    pub artifacts: BTreeMap<String, Artifact>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Collection {
    pub format: u32,
    pub namespace: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub extensions: Vec<Entry>,
}
impl Collection {
    pub fn id(&self) -> String {
        format!("{}.{}", self.namespace, self.name)
    }
    pub fn validate(&self) -> Result<()> {
        if self.format != 1 {
            bail!("Unsupported native collection format");
        }
        validate_name(&format!("{}.collection", self.id()))?;
        if self.version.is_empty()
            || self.version.len() > 64
            || !self
                .version
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
        {
            bail!("Invalid collection version identifier");
        }
        let mut names = std::collections::HashSet::new();
        for entry in &self.extensions {
            entry.definition.validate()?;
            if !entry
                .definition
                .name
                .starts_with(&format!("{}.", self.id()))
                || !names.insert((entry.definition.kind, entry.definition.name.clone()))
            {
                bail!("Native component namespace mismatch or duplicate name");
            }
            if entry.artifacts.is_empty() {
                bail!("Native component has no artifacts");
            }
            for (arch, artifact) in &entry.artifacts {
                if !matches!(arch.as_str(), "x86_64" | "aarch64") {
                    bail!("Unsupported native artifact architecture");
                }
                relative_path(&artifact.path)?;
                if artifact.sha256.len() != 64
                    || !artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    bail!("Invalid native artifact SHA-256");
                }
            }
        }
        Ok(())
    }
}
pub fn relative_path(path: &str) -> Result<std::path::PathBuf> {
    use std::path::Component;
    if path.is_empty() || path.len() > 4096 || path.contains(['\\', ':', '\0']) {
        bail!("Invalid native collection path");
    }
    let path = std::path::Path::new(path);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("Native collection path escapes its root");
    }
    Ok(path.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn definition() -> Definition {
        Definition {
            name: "example.native.inspect".into(),
            kind: Kind::Module,
            capabilities: Capabilities {
                check: true,
                diff: false,
                asynchronous: false,
                mutates: false,
            },
            parameters: BTreeMap::from([(
                "path".into(),
                Parameter {
                    kind: ParameterType::String,
                    required: true,
                },
            )]),
        }
    }
    #[test]
    fn schemas_preserve_types_and_reject_unknown_missing_or_unsupported_inputs() {
        let definition = definition();
        for invalid in [
            "{}",
            "{path: 42}",
            "{path: /etc/passwd, extra: true}",
            "[path]",
        ] {
            assert!(
                definition
                    .validate_call(&Call::new(
                        &definition,
                        serde_yaml::from_str(invalid).unwrap(),
                        true,
                        false,
                        100
                    ))
                    .is_err(),
                "{invalid}"
            );
        }
        let mut call = Call::new(
            &definition,
            serde_yaml::from_str("path: /etc/passwd").unwrap(),
            true,
            false,
            100,
        );
        assert!(definition.validate_call(&call).is_ok());
        call.diff = true;
        assert!(definition.validate_call(&call).is_err());
        call.diff = false;
        call.timeout_ms = 0;
        assert!(definition.validate_call(&call).is_err());
    }
    #[test]
    fn names_paths_providers_and_handshake_metadata_are_strict() {
        for invalid in [
            "shell",
            "ansible.builtin.shell",
            "example.native.Executable",
            "example.native.a/b",
        ] {
            assert!(validate_name(invalid).is_err());
        }
        for invalid in [
            "/etc/passwd",
            "../escape",
            "bin/../escape",
            "bin\\file",
            "bin:file",
            "",
        ] {
            assert!(relative_path(invalid).is_err(), "{invalid}");
        }
        let expected = definition();
        let mut hello = Hello {
            api_version: API_VERSION,
            wire_version: crate::VERSION,
            definition: expected.clone(),
        };
        assert!(hello.validate(&expected).is_ok());
        hello.definition.capabilities.mutates = true;
        assert!(hello.validate(&expected).is_err());
        let mut provider = expected;
        provider.kind = Kind::Inventory;
        provider.capabilities.mutates = true;
        assert!(provider.validate().is_err());
        provider.capabilities.mutates = false;
        provider.capabilities.asynchronous = true;
        assert!(provider.validate().is_err());
    }
}
