[English / 简体中文](../../../zh-CN/reference/modules/include_vars.md) · [Index / 目录](../../index.md)

# include_vars

<a id="purpose"></a>

## Purpose and location

Load controller variable files/directories at task time.

Controller. check: readonly; diff: false; async: false.

<a id="parameters"></a>

## Parameters

| Parameter | Type | Default / requirement | Meaning |
|---|---|---|---|
| `file` | string | — | Controller variable file, exclusive with dir. |
| `dir` | string | — | Controller variable directory, recursively read in lexical order. |
| `name` | string | — | Optional namespace for loaded values. |

<a id="behavior"></a>

## Validation and idempotence

Exactly one of file/dir. Optional name creates a namespace. Files follow the task source; directory merge is recursive and lexical. Decrypt Vault in memory.

<a id="returns"></a>

## Return values

ansible_facts, ansible_included_var_files. Absent/conditional fields must be tested before access. All modules use the [common result model](../results.md).

<a id="examples"></a>

## Example

The example is a complete playbook. Supply inventory, required variables/files and prepared target resources. Account/package/service examples require an isolated privileged target; example.invalid URLs are placeholders. Do not execute these blocks blindly. 

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - include_vars:
        file: vars.yml
```

<a id="verification"></a>

## Verification and related guides

Verify actual target state, then repeat an idempotent operation. Check predicts/skips and must leave business state unchanged; it does not make arbitrary commands idempotent. no_log hides presentation, not the in-memory register. See [support boundaries](../compatibility.md), [examples](../../../../examples/README.md) and [test layers](../../development/testing.md).

<!-- parameters: file,dir,name -->
