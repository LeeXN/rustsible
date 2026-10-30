[English / 简体中文](../../zh-CN/development/releasing.md) · [Index / 目录](../index.md)

# Build and verify offline packages

<a id="artifacts"></a>

## Freeze a source identity

Build the controller and x86_64/aarch64 workers from the same source with matching RUSTSIBLE_BUILD_ID. Use static musl targets for portable bundles. Package creation needs Cargo, tar and checksum tools; extracted execution does not. Never substitute a cross-compile for native architecture acceptance.

<a id="package"></a>

## Package layout and validation

```bash
bash scripts/package.sh CONTROLLER OUTPUT_DIRECTORY X86_64_WORKER AARCH64_WORKER
cd OUTPUT_DIRECTORY
sha256sum -c SHA256SUMS
./rustsible build-info
```

<a id="contents"></a>

## What is shipped

rustsible, workers/manifest.json and both workers, RELEASE.json, SHA256SUMS, both READMEs, docs/en and docs/zh-CN, curated examples and LICENSE. All packaged files except the checksum list itself are covered recursively. Source links become commit-pinned GitHub links in the package; documentation/example links remain local. No fixture credentials or acceptance binaries are shipped. SDK development still needs source/dependencies.

<a id="acceptance"></a>

## Final acceptance

Start the controller from the extracted archive. Validate disconnected minimal runtime, both native architectures and libc environments, three-server deployment, second-run idempotence, check snapshots, Vault, collection installation, handlers, health, byte-exact rollback, async and cleanup. release-target-evidence-ARCH artifacts retain source/build identity, target UID/GID/mode/SHA and zero-residual counts. Recheck the archive independently after download. CI success alone cannot cover undocumented backend combinations.
