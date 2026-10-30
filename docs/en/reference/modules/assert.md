[English / 简体中文](../../../zh-CN/reference/modules/assert.md) · [Index / 目录](../../index.md)

# assert

<a id="purpose"></a>

## Purpose and location

Evaluate strict boolean conditions on the controller.

Controller. check: readonly; diff: false; async: false.

<a id="parameters"></a>

## Parameters

| Parameter | Type | Default / requirement | Meaning |
|---|---|---|---|
| `that` | boolean/string/list | required | Boolean condition or expression; lists use AND. |
| `fail_msg` | string | Assertion failed | Failure text; msg is an alias. |
| `msg` | any/string | — | Message; assert/fail require string messages, debug can format structured values. |
| `success_msg` | string | All assertions passed | Success text. |
| `quiet` | boolean | false | Suppress normal successful assertion text. |

<a id="behavior"></a>

## Validation and idempotence

that required (boolean/expression/list). A list is AND. fail_msg/msg are exclusive aliases; quiet suppresses ordinary success messages. Never connects for the assertion itself.

<a id="returns"></a>

## Return values

assertion on failure, evaluated_to; common fields. Absent/conditional fields must be tested before access. All modules use the [common result model](../results.md).

<a id="examples"></a>

## Example

The example is a complete playbook. Supply inventory, required variables/files and prepared target resources. Account/package/service examples require an isolated privileged target; example.invalid URLs are placeholders. Do not execute these blocks blindly. 

```yaml
- hosts: all
  gather_facts: false
  vars:
    release_ready: true
  tasks:
    - assert:
        that:
          - release_ready is defined
          - release_ready
```

<a id="verification"></a>

## Verification and related guides

Verify actual target state, then repeat an idempotent operation. Check predicts/skips and must leave business state unchanged; it does not make arbitrary commands idempotent. no_log hides presentation, not the in-memory register. See [support boundaries](../compatibility.md), [examples](../../../../examples/README.md) and [test layers](../../development/testing.md).

<!-- parameters: that,fail_msg,msg,success_msg,quiet -->
