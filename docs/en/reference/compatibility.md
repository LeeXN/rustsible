[English / 简体中文](../../zh-CN/reference/compatibility.md) · [Index / 目录](../index.md)

# Supported scope and differences

<a id="platform"></a>

## Validated platforms

Linux x86_64/aarch64 control and targets, glibc/musl, native Rust workers and offline bundles. APT/APK, systemd/OpenRC/SysV have dual-architecture acceptance. DNF/Yum/Pacman/Zypper and Upstart have real x86_64 acceptance only; their ARM backend combinations are not marked verified. DNF/Yum may depend on OS Python.

<a id="reference"></a>

## Ansible reference

Supported semantics are compared with pinned ansible-core 2.21.0, not the entire ecosystem. FQCN aliases for builtins are supported; systemd_service aliases systemd. See the complete [module catalog](modules.md). No Windows/macOS, persistent agent, Galaxy Python module/plugin bridge, or stable Rust dynamic-library ABI is provided.

<a id="differences"></a>

## Explicit differences

Global --check cannot be disabled by check_mode:false. Temporary worker staging is allowed; business mutation and background launch are not. Task conditions require booleans; ordinary data is not compiled again. role argument_specs, task collections, vars_prompt, module_defaults, file.recurse, recursive copy, remote_src copy and unregistered filters such as regex_replace are unsupported. uri/wait_for skip check; command requires a prediction guard. Syntax/list operations are controller-only and dynamic content is selected at runtime.

<a id="limits"></a>

## Operational limits

Command output/request frames16 MiB; URI request/response and wait file matching8 MiB; lineinfile input64 MiB; extensions ELF256 MiB, stderr64 KiB; archives/collections8 GiB and100000 entries. Unknown params/combinations fail before business mutation. Current release and target evidence are described in [validation status](../development/validation-status.md).
