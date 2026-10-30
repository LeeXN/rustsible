[English / 简体中文](../../../zh-CN/reference/modules/service.md) · [Index / 目录](../../index.md)

# service

<a id="purpose"></a>

## Purpose and location

Manage running state and boot enablement via detected service tools.

Target worker. check: prediction; diff: false; async: true.

<a id="parameters"></a>

## Parameters

| Parameter | Type | Default / requirement | Meaning |
|---|---|---|---|
| `name` | string | required/conditional | Resource name; see module-specific requirements below. |
| `state` | string | unchanged | Desired resource state; accepted states are listed below. |
| `enabled` | boolean | unchanged | Desired boot enablement, independent of running state. |

<a id="behavior"></a>

## Validation and idempotence

name required and at least state or enabled. No default state is imposed. restarted/reloaded deliberately act each time. systemd/OpenRC/SysV/Upstart capabilities vary; unsupported enablement fails.

<a id="returns"></a>

## Return values

state/enabled when requested. Absent/conditional fields must be tested before access. All modules use the [common result model](../results.md).

<a id="examples"></a>

## Example

The example is a complete playbook. Supply inventory, required variables/files and prepared target resources. Account/package/service examples require an isolated privileged target; example.invalid URLs are placeholders. Do not execute these blocks blindly.

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - service:
        name: example-app
        state: started
        enabled: true
```

<a id="verification"></a>

## Verification and related guides

Verify actual target state, then repeat an idempotent operation. Check predicts/skips and must leave business state unchanged; it does not make arbitrary commands idempotent. no_log hides presentation, not the in-memory register. See [support boundaries](../compatibility.md), [examples](../../../../examples/README.md) and [test layers](../../development/testing.md).

<!-- parameters: name,state,enabled -->
