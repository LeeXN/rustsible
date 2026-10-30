This collection contains a read-only native module, inventory provider, lookup,
filter, role and playbook. Its executable is built from `crates/native-example`;
the crash and mutation acceptance peer is never included.

After building the programs for the supported target architectures, assemble a
source directory explicitly (the authoring tool never executes those programs):

```sh
cp -R examples/native-collection target/example-collection
cargo run --locked -p rustsible-native-example --example collection_manifest -- \
  target/example-collection \
  x86_64=target/x86_64-unknown-linux-musl/release/rustsible-example-extension \
  aarch64=target/aarch64-unknown-linux-musl/release/rustsible-example-extension
rustsible collection pack target/example-collection --output target/example-native.tar.gz
rustsible collection verify target/example-native.tar.gz
rustsible collection install target/example-native.tar.gz
rustsible playbook example.native.inspect -i inventory.yml
```

`example.native.inventory` accepts `path` to a JSON inventory with the same
`all/hosts/children/vars` structure as YAML inventory. Use
`lookup('example.native.record', '/absolute/record.json')` to read JSON data,
and `{{ title | example.native.slug }}` to produce an ASCII slug. These outputs
are data: their embedded template delimiters are never executed again.
