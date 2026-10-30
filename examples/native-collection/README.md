[English / 简体中文](README.zh-CN.md)

# Native collection example

This collection contains a readonly probe module, inventory, JSON record lookup, slug filter, role and playbook. Its executable source is crates/native-example; crash/write fixtures are not shipped. Build from a source checkout with dependencies, or use the matching collection archive for offline execution.

First run this on each matching Linux architecture with its musl linker tools, then collect the two programs at the paths below. Cross-compilation requires a configured target linker and is not native execution evidence.

```bash
native_target="$(uname -m)-unknown-linux-musl"
rustup target add "$native_target"
cargo build --locked -p rustsible-native-example --release --target "$native_target"
```

```bash
cp -R examples/native-collection target/example-collection
cargo run --locked -p rustsible-native-example --example collection_manifest -- target/example-collection x86_64=target/x86_64-unknown-linux-musl/release/rustsible-example-extension aarch64=target/aarch64-unknown-linux-musl/release/rustsible-example-extension
rustsible collection pack target/example-collection --output target/example.native.tar.gz
rustsible collection verify target/example.native.tar.gz
rustsible collection install target/example.native.tar.gz
rustsible playbook example.native.inspect -i inventory.yml
```

Build static programs for both architectures first; a single-architecture experiment may pass just its architecture. The manifest helper never runs packaged programs. Inventory accepts path to JSON matching the YAML inventory structure; lookup uses lookup('example.native.record', path), filter uses example.native.slug. Provider output is never re-templated.
