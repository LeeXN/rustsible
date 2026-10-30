[English / 简体中文](../../../zh-CN/reference/modules/lineinfile.md) · [Index / 目录](../../index.md)

# lineinfile

<a id="purpose"></a>

## Purpose and location

Ensure or remove a line while preserving safe file metadata.

Target worker. check: prediction; diff: true; async: true.

<a id="parameters"></a>

## Parameters

| Parameter | Type | Default / requirement | Meaning |
|---|---|---|---|
| `path` | string | required/conditional | Target path; file accepts dest as an alternative, never both. |
| `line` | string | required for present | Complete desired line, or exact line to remove without regexp. |
| `regexp` | string | — | Rust regular expression; Python-specific constructs are unsupported. |
| `state` | string | present | Desired resource state; accepted states are listed below. |
| `backup` | boolean | false | Create a protected backup when content changes. |
| `create` | boolean | false | Allow creation of a missing lineinfile target. |
| `insertafter` | string | EOF | Insertion anchor (regexp/EOF), using the last matching line. |
| `insertbefore` | string | — | Alternative insertion anchor (regexp/BOF); exclusive with insertafter. |
| `owner` | string | preserve / execution identity | Target owner name or numeric ID string; resolved before mutation. |
| `group` | string | preserve / execution identity | Target group name/ID; user uses this as a primary-group name. |
| `mode` | string | preserve / umask | Quoted octal permissions; preserve an existing file when omitted. |

<a id="behavior"></a>

## Validation and idempotence

state defaults present; present requires line, absent requires line or regexp. Last match wins, otherwise insert at selected anchor/EOF. Input is UTF-8, at most64 MiB. backup preserves protected metadata. No Python-regex backreferences.

<a id="returns"></a>

## Return values

backup_file when created; common fields. Absent/conditional fields must be tested before access. All modules use the [common result model](../results.md).

<a id="examples"></a>

## Example

The example is a complete playbook. Supply inventory, required variables/files and prepared target resources. Account/package/service examples require an isolated privileged target; example.invalid URLs are placeholders. Do not execute these blocks blindly. 

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - lineinfile:
        path: '{{ demo_root }}/config'
        line: enabled=true
        create: true
```

<a id="verification"></a>

## Verification and related guides

Verify actual target state, then repeat an idempotent operation. Check predicts/skips and must leave business state unchanged; it does not make arbitrary commands idempotent. no_log hides presentation, not the in-memory register. See [support boundaries](../compatibility.md), [examples](../../../../examples/README.md) and [test layers](../../development/testing.md).

<!-- parameters: path,line,regexp,state,backup,create,insertafter,insertbefore,owner,group,mode -->
