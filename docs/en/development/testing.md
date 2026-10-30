[English / 简体中文](../../zh-CN/development/testing.md) · [Index / 目录](../index.md)

# Testing and acceptance

<a id="layers"></a>

## What each layer proves

Unit tests cover parsing, validation, state and protocol. Local integrations inspect real files, scopes, loops, retries and privacy. SSH fixtures exercise strict trust, password/key auth, three identities, disconnects, timeout and cleanup. Real service/package fixtures validate OS tools, not mock success. A line-coverage floor is a regression gate, not semantic proof.

<a id="local"></a>

## Run local checks

Build the workspace before integration/coverage so standalone workers and SDK peers exist. Tests create private temporary resources; ordinary Cargo tests do not require Python. Use the [contribution checks](contributing.md#checks). Documentation tests run curated local examples twice and compare check snapshots.

<a id="ssh"></a>

## Disposable SSH and OS fixtures

```bash
bash tests/ssh/run.sh CONTROLLER WORKERS
bash tests/ssh/run.sh CONTROLLER WORKERS reuse musl
bash tests/ssh/run.sh CONTROLLER WORKERS strategy glibc
bash tests/ssh/run.sh CONTROLLER WORKERS async glibc
bash tests/ssh/run.sh CONTROLLER WORKERS network glibc HTTP_FIXTURE
bash tests/managers/run.sh CONTROLLER WORKERS dnf
bash tests/managers/upstart.sh CONTROLLER WORKERS HTTP_FIXTURE
```

<a id="reference"></a>

## Reference and evidence

Replace uppercase paths with matching binaries. Build http_fixture from the worker example (static for minimal images); prompt_fixture is a controller example used by operator/Vault modes. The pinned reference command is bash tests/reference/run.sh CONTROLLER HTTP_FIXTURE; it uses uv and ansible-core==2.21.0 separately from the product. Native ARM and x86 jobs prove their own architecture; QEMU-controller cross cases are separately labeled. Check target bytes/mode/owner/digests and owned residuals, not only rc/changed.
