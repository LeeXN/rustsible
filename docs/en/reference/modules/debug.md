[English / 简体中文](../../../zh-CN/reference/modules/debug.md) · [Index / 目录](../../index.md)

# debug

<a id="purpose"></a>

## Purpose and location

Display resolved controller data without connecting.

Controller. check: readonly; diff: false; async: false.

<a id="parameters"></a>

## Parameters

| Parameter | Type | Default / requirement | Meaning |
|---|---|---|---|
| `msg` | any/string | — | Message; assert/fail require string messages, debug can format structured values. |
| `var` | any | — | Variable expression resolved on the controller, Prefer one of msg/var; msg takes precedence. |

<a id="behavior"></a>

## Validation and idempotence

Supply msg or var; if both are supplied this implementation displays msg. A variable name may be used without template delimiters. No resource changes. Protect sensitive debug output with no_log.

<a id="returns"></a>

## Return values

msg, stdout. Absent/conditional fields must be tested before access. All modules use the [common result model](../results.md).

<a id="examples"></a>

## Example

The example is a complete playbook. Supply inventory, required variables/files and prepared target resources. Account/package/service examples require an isolated privileged target; example.invalid URLs are placeholders. Do not execute these blocks blindly. 

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - debug:
        msg: hello
```

<a id="verification"></a>

## Verification and related guides

Verify actual target state, then repeat an idempotent operation. Check predicts/skips and must leave business state unchanged; it does not make arbitrary commands idempotent. no_log hides presentation, not the in-memory register. See [support boundaries](../compatibility.md), [examples](../../../../examples/README.md) and [test layers](../../development/testing.md).

<!-- parameters: msg,var -->
