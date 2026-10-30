[English / 简体中文](../../../zh-CN/reference/modules/meta.md) · [Index / 目录](../../index.md)

# meta

<a id="purpose"></a>

## Purpose and location

Control playbook execution and handler phases.

Controller. check: readonly; diff: false; async: false.

<a id="parameters"></a>

## Parameters

| Parameter | Type | Default / requirement | Meaning |
|---|---|---|---|
| `action` | string | required | flush_handlers, clear_host_errors, reset_connection, end_host or end_play. |

<a id="behavior"></a>

## Validation and idempotence

flush_handlers runs pending notifications; clear_host_errors reenables hosts; reset_connection drops the lease; end_host/end_play end execution. Ad-hoc supports reset_connection only.

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
    - meta:
        action: flush_handlers
```

<a id="verification"></a>

## Verification and related guides

Verify actual target state, then repeat an idempotent operation. Check predicts/skips and must leave business state unchanged; it does not make arbitrary commands idempotent. no_log hides presentation, not the in-memory register. See [support boundaries](../compatibility.md), [examples](../../../../examples/README.md) and [test layers](../../development/testing.md).

<!-- parameters: action -->
