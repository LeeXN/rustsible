//! A native collection can supply several components from one executable.
//! The published example is read-only; mutation/crash fixtures are separate.
use anyhow::{bail, Context, Result};
use rustsible_protocol::{
    extension::{Call, Capabilities, Definition, Kind, Parameter, ParameterType as T},
    ModuleResult,
};
use serde_yaml::Value;
use std::{collections::BTreeMap, io::Read, os::unix::fs::OpenOptionsExt};
fn definition(
    name: &str,
    kind: Kind,
    parameters: &[(&str, T, bool)],
    mutates: bool,
    diff: bool,
    asynchronous: bool,
) -> Definition {
    Definition {
        name: name.into(),
        kind,
        capabilities: Capabilities {
            check: true,
            diff,
            asynchronous,
            mutates,
        },
        parameters: parameters
            .iter()
            .map(|(name, kind, required)| {
                (
                    (*name).into(),
                    Parameter {
                        kind: *kind,
                        required: *required,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>(),
    }
}
pub fn catalog() -> Vec<Definition> {
    vec![
        definition(
            "example.native.probe",
            Kind::Module,
            &[("path", T::String, true), ("delay_ms", T::Integer, false)],
            false,
            false,
            true,
        ),
        definition(
            "example.native.inventory",
            Kind::Inventory,
            &[("path", T::String, true)],
            false,
            false,
            false,
        ),
        definition(
            "example.native.record",
            Kind::Lookup,
            &[("terms", T::List, true), ("keywords", T::Mapping, true)],
            false,
            false,
            false,
        ),
        definition(
            "example.native.slug",
            Kind::Filter,
            &[
                ("value", T::Any, true),
                ("arguments", T::List, true),
                ("keywords", T::Mapping, true),
            ],
            false,
            false,
            false,
        ),
    ]
}
pub fn fixture_catalog() -> Vec<Definition> {
    let mut values = catalog();
    values.extend([
        definition(
            "fixture.native.write",
            Kind::Module,
            &[("path", T::String, true), ("content", T::String, true)],
            true,
            true,
            true,
        ),
        definition(
            "fixture.native.crash",
            Kind::Module,
            &[],
            true,
            false,
            false,
        ),
        definition(
            "fixture.native.timeout",
            Kind::Module,
            &[("path", T::String, true)],
            true,
            false,
            true,
        ),
    ]);
    values
}
fn read_json(path: &str) -> Result<Value> {
    let mut bytes = Vec::new();
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        bail!("Example JSON input must be a regular file");
    }
    file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        bail!("Example JSON input exceeds 1 MiB");
    }
    Ok(serde_yaml::to_value(serde_json::from_slice::<
        serde_json::Value,
    >(&bytes)?)?)
}
pub fn handle(definition: &Definition, call: &Call) -> Result<ModuleResult> {
    let string = |name| {
        call.args
            .get(name)
            .and_then(Value::as_str)
            .context("Expected example string argument")
    };
    let mut result = ModuleResult {
        msg: "Native example completed".into(),
        ..Default::default()
    };
    match definition.name.as_str() {
        "example.native.probe" => {
            if let Some(value) = call.args.get("delay_ms") {
                let delay = value
                    .as_u64()
                    .filter(|delay| *delay <= 60000)
                    .context("delay_ms must be between 0 and 60000")?;
                std::thread::sleep(std::time::Duration::from_millis(delay));
            }
            let metadata = match std::fs::metadata(string("path")?) {
                Ok(metadata) => Some(metadata),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            };
            result.values.insert("probe".into(),serde_yaml::to_value(serde_json::json!({"exists":metadata.is_some(),"bytes":metadata.as_ref().map(|metadata|metadata.len()),"uid":unsafe{libc::geteuid()},"arch":std::env::consts::ARCH}))?);
        }
        "example.native.inventory" => {
            result
                .values
                .insert("inventory".into(), read_json(string("path")?)?);
        }
        "example.native.record" => {
            if call.args["terms"].as_sequence().map(Vec::len) != Some(1)
                || call.args["keywords"]
                    .as_mapping()
                    .is_none_or(|value| !value.is_empty())
            {
                bail!("Record requires one path and no keyword arguments");
            }
            let path = call.args["terms"]
                .as_sequence()
                .and_then(|values| values.first())
                .and_then(Value::as_str)
                .context("Record lookup requires a JSON path")?;
            result.values.insert("value".into(), read_json(path)?);
        }
        "example.native.slug" => {
            if call.args["arguments"]
                .as_sequence()
                .is_none_or(|value| !value.is_empty())
                || call.args["keywords"]
                    .as_mapping()
                    .is_none_or(|value| !value.is_empty())
            {
                bail!("Slug accepts no additional arguments");
            }
            let input = call.args["value"]
                .as_str()
                .context("Slug requires a string")?;
            let slug = input
                .chars()
                .map(|character| {
                    if character.is_ascii_alphanumeric() {
                        character.to_ascii_lowercase()
                    } else {
                        '-'
                    }
                })
                .collect::<String>();
            result.values.insert("value".into(), Value::String(slug));
        }
        _ => bail!("Unknown example component"),
    }
    Ok(result)
}
