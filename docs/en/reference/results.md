[English / 简体中文](../../zh-CN/reference/results.md) · [Index / 目录](../index.md)

# Results, registers and failures

<a id="fields"></a>

## Common fields

changed, failed, skipped, unreachable, uncertain and msg describe execution. rc/stdout/stderr and line arrays apply when commands return them. Modules may add values documented on their page. Test optional fields before access. A failed nonzero command keeps its rc and output. Failure after a known change retains changed:true.

<a id="ownership"></a>

## Registration and ownership

register updates after every retry; loops collect per-item results and attempts. Delegation changes execution target but keeps the logical host register unless delegate_facts explicitly moves facts. Data is never auto-templated again. failed_when/changed_when evaluate against the current result; notifications follow final changed/failed semantics.

<a id="failure"></a>

## Failure boundaries

Unreachable is a connection failure; uncertain means a modifying request may have run but its result cannot be confirmed. Neither is a safe automatic replay. Failed hosts stay excluded across plays until cleared. Ordinary failures can enter rescue; ignore_errors does not erase the registered failure.

<a id="display"></a>

## Display is not storage

no_log censors presentation, not the raw in-memory register. Async privacy persists in status records. Diff strings over64 KiB or containing binary NUL are summarized; aggregate diff is bounded too. Vault input suppresses sensitive diagnostics/diff. Never use debug to publish a secret merely because its source is encrypted.
