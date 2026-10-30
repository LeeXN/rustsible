[English / 简体中文](../../zh-CN/reference/extension-protocol.md) · [Index / 目录](../index.md)

# Native executable extension API v1

<a id="transport"></a>

## Transport and trust

Extensions are explicitly trusted ELF executables, not a sandbox. stdin/stdout carry big-endian u32-length JSON frames; stderr is bounded diagnostics, never a second result channel. Probe identifies the FQCN and kind; Hello must match API version, wire version, schema and capabilities before Call delivers arguments. Reply contains API version and ModuleResult. Extension builds do not need the controller Git BUILD_ID, but must match API/wire/schema.

<a id="manifest"></a>

## Collection manifest

rustsible-collection.json has format1, namespace/name/version and extensions. Each entry includes definition (FQCN, kind, capabilities check/diff/async/mutates, parameter types/required) and artifacts indexed by x86_64/aarch64 with relative path/SHA256. Reserved builtin namespaces cannot be replaced. Modules select the real target architecture; inventory/lookup/filter require the controller architecture.

<a id="providers"></a>

## Provider contracts

module runs in the target worker. inventory accepts configured args and returns values.inventory in YAML-inventory-compatible JSON shape, not the legacy _meta script protocol. lookup receives terms and keywords and returns values.value; filter receives value/arguments/keywords and returns values.value. Nonmodule providers must declare readonly/check and no diff/async. Their output is literal typed data.

<a id="limits"></a>

## Bounds and lifecycle

Request/result16 MiB, stderr64 KiB, ELF256 MiB. Programs must be owned by root/current UID and not group/other-writable, with safe collection directories. Verify hashes/ELF on every execution. A crash after a mutating request may be uncertain and is never automatically replayed. Supervisors reclaim detached descendants; async retains private program copies until job cleanup. Use the [SDK guide](../development/native-extensions.md).
