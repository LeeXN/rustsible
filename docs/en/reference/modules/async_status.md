[English / 简体中文](../../../zh-CN/reference/modules/async_status.md) · [Index / 目录](../../index.md)

# async_status

<a id="purpose"></a>

## Purpose and location

Query or clean a job owned by the execution identity.

Target worker. check: prediction; diff: false; async: false.

<a id="parameters"></a>

## Parameters

| Parameter | Type | Default / requirement | Meaning |
|---|---|---|---|
| `jid` | string | required | Opaque j followed by 64 lowercase hex digits; not a path. |
| `mode` | string | status | status or cleanup; cleanup predicts deletion in check. |

<a id="behavior"></a>

## Validation and idempotence

jid required, mode status/cleanup defaults status. ansible_async_dir selects directory through connection variables. Cleanup removes only a completed safe job; cannot inspect another identity by guessing an ID. Polling failure/no_log remains preserved.

<a id="returns"></a>

## Return values

ansible_job_id, started, finished; completed module fields. Absent/conditional fields must be tested before access. All modules use the [common result model](../results.md).

<a id="examples"></a>

## Example

The example is a complete playbook. Supply inventory, required variables/files and prepared target resources. Account/package/service examples require an isolated privileged target; example.invalid URLs are placeholders. Do not execute these blocks blindly.

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - async_status:
        jid: '{{ background.ansible_job_id }}'
        mode: status
```

<a id="verification"></a>

## Verification and related guides

Verify actual target state, then repeat an idempotent operation. Check predicts/skips and must leave business state unchanged; it does not make arbitrary commands idempotent. no_log hides presentation, not the in-memory register. See [support boundaries](../compatibility.md), [examples](../../../../examples/README.md) and [test layers](../../development/testing.md).

<!-- parameters: jid,mode -->
