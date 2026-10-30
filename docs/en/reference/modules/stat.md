[English / 简体中文](../../../zh-CN/reference/modules/stat.md) · [Index / 目录](../../index.md)

# stat

<a id="purpose"></a>

## Purpose and location

Inspect target metadata and regular-file checksums without following links.

Target worker. check: readonly; diff: false; async: false.

<a id="parameters"></a>

## Parameters

| Parameter | Type | Default / requirement | Meaning |
|---|---|---|---|
| `path` | string | required/conditional | Target path; file accepts dest as an alternative, never both. |
| `checksum_algorithm` | string | sha1 | md5, sha1, sha224, sha256, sha384 or sha512. |

<a id="behavior"></a>

## Validation and idempotence

path required, checksum_algorithm defaults sha1. Missing paths return stat.exists:false; checksum is present only for stable readable regular files. No unsupported checksum options.

<a id="returns"></a>

## Return values

stat: exists; when present isreg/isdir/islnk/mode/uid/gid/size/inode/dev/nlink/mtime/ctime; regular-file checksum. Absent/conditional fields must be tested before access. All modules use the [common result model](../results.md).

<a id="examples"></a>

## Example

The example is a complete playbook. Supply inventory, required variables/files and prepared target resources. Account/package/service examples require an isolated privileged target; example.invalid URLs are placeholders. Do not execute these blocks blindly.

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - stat:
        path: /etc/passwd
        checksum_algorithm: sha256
```

<a id="verification"></a>

## Verification and related guides

Verify actual target state, then repeat an idempotent operation. Check predicts/skips and must leave business state unchanged; it does not make arbitrary commands idempotent. no_log hides presentation, not the in-memory register. See [support boundaries](../compatibility.md), [examples](../../../../examples/README.md) and [test layers](../../development/testing.md).

<!-- parameters: path,checksum_algorithm -->
