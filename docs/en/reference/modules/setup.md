[English / 简体中文](../../../zh-CN/reference/modules/setup.md) · [Index / 目录](../../index.md)

# setup

<a id="purpose"></a>

## Purpose and location

Collect facts from the execution target.

Target worker. check: readonly; diff: false; async: false.

<a id="parameters"></a>

## Parameters

| Parameter | Type | Default / requirement | Meaning |
|---|---|---|---|
| `gather_subset` | string/list[string] | all | Supported subset names and ! exclusions; see facts reference. |
| `filter` | string/list[string] | all returned keys | Shell patterns applied to top-level returned fact names. |
| `gather_timeout` | integer | 10 | Target fact collection budget, 1–604800 seconds. |

<a id="behavior"></a>

## Validation and idempotence

Supports named subsets and top-level filters. !all keeps minimum, !all,!min starts empty. Explicit setup merges facts like automatic gathering. No Python/local executable fact collectors.

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
    - setup:
        gather_subset: [platform, distribution]
        filter: [ansible_architecture, ansible_distribution]
```

<a id="verification"></a>

## Verification and related guides

Verify actual target state, then repeat an idempotent operation. Check predicts/skips and must leave business state unchanged; it does not make arbitrary commands idempotent. no_log hides presentation, not the in-memory register. See [support boundaries](../compatibility.md), [examples](../../../../examples/README.md) and [test layers](../../development/testing.md).

<!-- parameters: gather_subset,filter,gather_timeout -->
