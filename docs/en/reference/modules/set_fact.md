[English / 简体中文](../../../zh-CN/reference/modules/set_fact.md) · [Index / 目录](../../index.md)

# set_fact

<a id="purpose"></a>

## Purpose and location

Publish typed persistent host values.

Controller. check: readonly; diff: false; async: false.

<a id="parameters"></a>

## Parameters

| Parameter | Type | Default / requirement | Meaning |
|---|---|---|---|
| `cacheable` | boolean | false | Only false is accepted; persistent fact caching is unsupported. |
| `<variable>` | any | required | One or more valid variable names with typed values. |

<a id="behavior"></a>

## Validation and idempotence

Requires at least one variable. Values survive plays in this run; not a disk cache. cacheable:true is rejected. Host variable publication is available in check.

<a id="returns"></a>

## Return values

ansible_facts. Absent/conditional fields must be tested before access. All modules use the [common result model](../results.md).

<a id="examples"></a>

## Example

The example is a complete playbook. Supply inventory, required variables/files and prepared target resources. Account/package/service examples require an isolated privileged target; example.invalid URLs are placeholders. Do not execute these blocks blindly.

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - set_fact:
        release_ready: true
```

<a id="verification"></a>

## Verification and related guides

Verify actual target state, then repeat an idempotent operation. Check predicts/skips and must leave business state unchanged; it does not make arbitrary commands idempotent. no_log hides presentation, not the in-memory register. See [support boundaries](../compatibility.md), [examples](../../../../examples/README.md) and [test layers](../../development/testing.md).

<!-- parameters: cacheable,<variable> -->
