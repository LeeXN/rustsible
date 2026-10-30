[English / 简体中文](../../../zh-CN/reference/modules/package.md) · [Index / 目录](../../index.md)

# package

<a id="purpose"></a>

## Purpose and location

Manage concrete package names using the detected target manager.

Target worker. check: prediction; diff: false; async: true.

<a id="parameters"></a>

## Parameters

| Parameter | Type | Default / requirement | Meaning |
|---|---|---|---|
| `name` | string/list[string] | required/conditional | Resource name; see module-specific requirements below. |
| `state` | string | present | Desired resource state; accepted states are listed below. |
| `update_cache` | boolean | false | Refresh package metadata before candidate queries; never refresh in check. |

<a id="behavior"></a>

## Validation and idempotence

name string/list required; state present/absent/latest defaults present. update_cache:false by default; refresh before candidate queries. Check uses existing cache only. Zypper latest is rejected; APT name=version is supported. Availability depends on actual tools.

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
    - package:
        name: [git, vim]
        state: present
        update_cache: true
```

<a id="verification"></a>

## Verification and related guides

Verify actual target state, then repeat an idempotent operation. Check predicts/skips and must leave business state unchanged; it does not make arbitrary commands idempotent. no_log hides presentation, not the in-memory register. See [support boundaries](../compatibility.md), [examples](../../../../examples/README.md) and [test layers](../../development/testing.md).

<!-- parameters: name,state,update_cache -->
