[English / 简体中文](../../../zh-CN/reference/modules/group.md) · [Index / 目录](../../index.md)

# group

<a id="purpose"></a>

## Purpose and location

Manage Linux groups with idempotent target queries.

Target worker. check: prediction; diff: false; async: false.

<a id="parameters"></a>

## Parameters

| Parameter | Type | Default / requirement | Meaning |
|---|---|---|---|
| `name` | string | required/conditional | Resource name; see module-specific requirements below. |
| `gid` | integer | existing / OS allocation | Nonnegative target GID; user cannot specify both gid and group. |
| `state` | string | present | Desired resource state; accepted states are listed below. |
| `system` | boolean | false | Create a system account/group; does not convert an existing resource. |

<a id="behavior"></a>

## Validation and idempotence

name required, state present/absent defaults present. system defaults false and applies on creation. Reject incompatible GID/name changes; groupadd/groupmod/groupdel are required.

<a id="returns"></a>

## Return values

name, state, gid when available. Absent/conditional fields must be tested before access. All modules use the [common result model](../results.md).

<a id="examples"></a>

## Example

The example is a complete playbook. Supply inventory, required variables/files and prepared target resources. Account/package/service examples require an isolated privileged target; example.invalid URLs are placeholders. Do not execute these blocks blindly. 

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - group:
        name: example_app
        state: present
        system: true
```

<a id="verification"></a>

## Verification and related guides

Verify actual target state, then repeat an idempotent operation. Check predicts/skips and must leave business state unchanged; it does not make arbitrary commands idempotent. no_log hides presentation, not the in-memory register. See [support boundaries](../compatibility.md), [examples](../../../../examples/README.md) and [test layers](../../development/testing.md).

<!-- parameters: name,gid,state,system -->
