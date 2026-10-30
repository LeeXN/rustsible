[English / 简体中文](../../zh-CN/tutorials/install.md) · [Index / 目录](../index.md)

# Install and verify

<a id="requirements"></a>

## Choose the correct release

Control and targets support Linux x86_64 and aarch64. Choose the controller matching the control machine. Each offline bundle contains both static workers. Targets need SSH for remote access, POSIX sh, uname, id, stat, mktemp, head, wc, sha256sum, cut, chmod and rm. Become needs sudo; module-specific OS tools are separate requirements. No Python or curl is required by Rustsible.

<a id="verify"></a>

## Extract before executing

Download the matching archive and the optional native collection from the same CI/release artifact. Replace x86_64 below with aarch64 on ARM. Keep rustsible beside workers/. RELEASE.json records the source and binary hashes; build-info does not read inventory or configuration.

```bash
tar -xzf x86_64.tar.gz
cd x86_64
sha256sum -c SHA256SUMS
./rustsible build-info
export PATH="$PWD:$PATH"
```

<a id="source"></a>

## Build from source

For development, build the whole workspace before tests. A host build supplies only the host worker; a dual-architecture package needs both workers built from the same source. Source archives without Git require RUSTSIBLE_BUILD_ID. Do not mix independently built controllers and workers.

```bash
cargo build --workspace --all-features --locked
cargo run --locked -p rustsible-protocol --example worker_manifest -- target/debug/workers "$(uname -m)=target/debug/rustsible-worker"
target/debug/rustsible build-info
```

<a id="next"></a>

## Next

Continue with [local deployment](local.md), then [SSH](ssh.md). The archive includes this documentation and the examples; developing the SDK additionally needs source and cached Cargo dependencies.
