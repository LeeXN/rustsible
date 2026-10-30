[English / 简体中文](../../../zh-CN/reference/modules/file.md) · [Index / 目录](../../index.md)

# file

<a id="purpose"></a>

## Purpose and location

Manage native files, directories, links and metadata.

Target worker. check: prediction; diff: true; async: true.

<a id="parameters"></a>

## Parameters

| Parameter | Type | Default / requirement | Meaning |
|---|---|---|---|
| `path` | string | required/conditional | Target path; file accepts dest as an alternative, never both. |
| `dest` | string | required/conditional | Target destination; parent must be prepared for content installation. |
| `state` | string | file | Desired resource state; accepted states are listed below. |
| `src` | string | — | Symlink target, required for state:link. |
| `mode` | string | preserve / umask | Quoted octal permissions; preserve an existing file when omitted. |
| `owner` | string | preserve / execution identity | Target owner name or numeric ID string; resolved before mutation. |
| `group` | string | preserve / execution identity | Target group name/ID; user uses this as a primary-group name. |

<a id="behavior"></a>

## Validation and idempotence

Exactly one of path/dest. Default state=file does not create missing content. States directory/link/absent/touch/file; link requires src. Recursive creation of missing parents is supported, recursive metadata repair is not. touch intentionally changes timestamps.

<a id="returns"></a>

## Return values

common result fields. Absent/conditional fields must be tested before access. All modules use the [common result model](../results.md).

<a id="examples"></a>

## Example

The example is a complete playbook. Supply inventory, required variables/files and prepared target resources. Account/package/service examples require an isolated privileged target; example.invalid URLs are placeholders. Do not execute these blocks blindly. 

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - file:
        path: '{{ demo_root }}/data'
        state: directory
        mode: '0700'
```

<a id="verification"></a>

## Verification and related guides

Verify actual target state, then repeat an idempotent operation. Check predicts/skips and must leave business state unchanged; it does not make arbitrary commands idempotent. no_log hides presentation, not the in-memory register. See [support boundaries](../compatibility.md), [examples](../../../../examples/README.md) and [test layers](../../development/testing.md).

<!-- parameters: path,dest,state,src,mode,owner,group -->
