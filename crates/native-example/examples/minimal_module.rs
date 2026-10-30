//! Minimal read-only module used verbatim in the bilingual SDK guide.
use anyhow::Result;
use rustsible_protocol::{
    extension::{serve, Capabilities, Definition, Kind, Parameter, ParameterType},
    ModuleResult,
};
use std::collections::{BTreeMap, HashMap};
fn main() -> Result<()> {
    let definition = Definition {
        name: "example.native.exists".into(),
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
    };
    serve(&[definition], |_, call| {
        let path = call.args["path"]
            .as_str()
            .expect("SDK validates the schema");
        let exists = match std::fs::metadata(path) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.into()),
        };
        Ok(ModuleResult {
            values: HashMap::from([("exists".into(), serde_yaml::Value::Bool(exists))]),
            ..Default::default()
        })
    })
}
