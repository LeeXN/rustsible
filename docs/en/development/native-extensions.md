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

<a id="build"></a>

## Build and package

From a source checkout with dependencies, build static binaries for supported architectures; the authoring helper inspects bytes without running the package program. Include roles/playbooks as declarative content and install explicitly before use. The sample instructions are shared in [the collection example](../../../examples/native-collection/README.md).

<a id="contracts"></a>

## Implement the contract

Declare strict parameters and truthful capabilities. Honor Call.check before OS mutation and no_log for sensitive results. Return structured errors without echoing input secrets. Keep stdout protocol-only. Do not implement a Python bridge or execute discovered repository programs. Test unknown args/types, failed handshake before parameter delivery, check invariance, second-run idempotence, crash/timeout, async privacy and actual target architecture. See [protocol](../reference/extension-protocol.md).
