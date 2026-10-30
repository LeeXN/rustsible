[English / 简体中文](../../../zh-CN/reference/modules/fail.md) · [Index / 目录](../../index.md)

# fail

<a id="purpose"></a>

## Purpose and location

Return an intentional controller-side task failure.

Controller. check: readonly; diff: false; async: false.

<a id="parameters"></a>

## Parameters

| Parameter | Type | Default / requirement | Meaning |
|---|---|---|---|
| `msg` | any/string | — | Message; assert/fail require string messages, debug can format structured values. |

<a id="behavior"></a>

## Validation and idempotence

Optional string msg; use in selected conditions or rescue. no_log still censors it. Does not modify a target resource.

<a id="returns"></a>

## Return values

common failed result. Absent/conditional fields must be tested before access. All modules use the [common result model](../results.md).

<a id="examples"></a>

## Example

The example is a complete playbook. Supply inventory, required variables/files and prepared target resources. Account/package/service examples require an isolated privileged target; example.invalid URLs are placeholders. Do not execute these blocks blindly.

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - fail:
        msg: Deployment precondition was not met
```

<a id="verification"></a>

## Verification and related guides

Verify actual target state, then repeat an idempotent operation. Check predicts/skips and must leave business state unchanged; it does not make arbitrary commands idempotent. no_log hides presentation, not the in-memory register. See [support boundaries](../compatibility.md), [examples](../../../../examples/README.md) and [test layers](../../development/testing.md).

<!-- parameters: msg -->
