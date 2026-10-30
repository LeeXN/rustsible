[English / 简体中文](../../zh-CN/development/contributing.md) · [Index / 目录](../index.md)

# Contributing changes

<a id="workflow"></a>

## Work from behavior

Read all callers, shared validators and the relevant tests before editing. Fix shared behavior once rather than adding frontend-specific patches. The Rust API is0.1 and can change deliberately; executable extension API/version checks are explicit. Keep secrets out of argv/logs and never use a private deployment as an implicit test target.

<a id="checks"></a>

## Required checks

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo build --workspace --all-features --locked
cargo test --workspace --all-targets --all-features --locked
cargo run --locked --example docs -- --check
cargo llvm-cov --workspace --all-targets --all-features --locked --fail-under-lines 80
cargo build --workspace --release --all-features --locked
cargo audit
```

<a id="docs"></a>

## Maintain both languages

Every public Markdown page has a peer. Update both with the same section IDs, executable blocks, accepted params/defaults and example references. Generate CLI/catalog with --write, commit both results, and run --check. Do not automatically translate identifiers or claim untested compatibility. Keep LICENSE authoritative and AGENTS.md outside documentation tooling.
