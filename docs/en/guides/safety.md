[English / 简体中文](../../zh-CN/guides/safety.md) · [Index / 目录](../index.md)

# Safety and sensitive data

<a id="check"></a>

## Predict business changes

Global --check cannot be canceled by task check_mode:false. It may stage/clean a temporary worker but must not update business files, accounts, packages, services or launch async. Some modules skip rather than predict; inspect their reference. Syntax/list operations never connect to the target.

<a id="identity"></a>

## Trust and identity

Verify SSH fingerprints through a trusted channel, keep private keys out of inventory repositories, and use hidden prompts/password sources. Distinguish login and become users. Worker/request/job directories and files are private; choosing a shared world-readable directory is not an identity-sharing mechanism. Native collection installation/configuration authorizes code with the selected identity, not sandboxed code. SHA256 verifies integrity, not author authenticity.

<a id="secrets"></a>

## Vault, no_log and output

Vault encryption protects stored configuration. CLI view/decrypt and intentional template/copy deployment expose the requested plaintext destination. no_log hides result presentation, not raw registers in memory, and does not make a secret safe to send to debug or a third-party program. Keep credentials out of argv; do not print variables to diagnose encrypted content. Private async results stay private on later queries.

<a id="failure"></a>

## Atomic writes and uncertain results

Builtin content installation prepares same-filesystem staging and preserves unspecified metadata before commit. Failed checksums/validation/writes keep original content; no unsafe truncate fallback exists. Arbitrary shell commands have only the guarantees their author implements. If connectivity is lost after a modifying request, inspect target state before deciding what to run next; uncertain is not permission to replay.
