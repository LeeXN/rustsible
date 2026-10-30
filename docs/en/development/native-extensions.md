[English / 简体中文](../../zh-CN/development/native-extensions.md) · [Index / 目录](../index.md)

# Author native extensions

<a id="example"></a>

## Start with the shipped SDK example

The complete example source is [crates/native-example](https://github.com/LeeXN/rustsible/tree/main/crates/native-example). It supplies readonly probe, inventory, JSON record lookup and slug filter from one executable. The crash/write fixture is for acceptance only and is not a published collection.

<a id="serve"></a>

## SDK entrypoint

```rust
fn main() -> anyhow::Result<()> {
    rustsible_protocol::extension::serve(
        &rustsible_native_example::catalog(),
        rustsible_native_example::handle,
    )
}
```

<a id="minimal"></a>

## Minimal custom module

This complete module declares its own strict schema and returns an exists boolean. It treats permission errors as errors rather than missing files. It is compiled as the minimal_module example of rustsible-native-example; both language pages include the same tested source.

```rust
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
```

<a id="build"></a>

## Build and package

From a source checkout with dependencies, build static binaries for supported architectures; the authoring helper inspects bytes without running the package program. Include roles/playbooks as declarative content and install explicitly before use. The sample instructions are shared in [the collection example](../../../examples/native-collection/README.md).

<a id="contracts"></a>

## Implement the contract

Declare strict parameters and truthful capabilities. Honor Call.check before OS mutation and no_log for sensitive results. Return structured errors without echoing input secrets. Keep stdout protocol-only. Do not implement a Python bridge or execute discovered repository programs. Test unknown args/types, failed handshake before parameter delivery, check invariance, second-run idempotence, crash/timeout, async privacy and actual target architecture. See [protocol](../reference/extension-protocol.md).
