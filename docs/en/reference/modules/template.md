[English / 简体中文](../../../zh-CN/reference/modules/template.md) · [Index / 目录](../../index.md)

# template

<a id="purpose"></a>

## Purpose and location

Render MiniJinja on the controller and install bytes atomically.

Controller rendering, target installation. check: prediction; diff: true; async: false.

<a id="parameters"></a>

## Parameters

| Parameter | Type | Default / requirement | Meaning |
|---|---|---|---|
| `src` | string | — | Source location; interpretation depends on the module. |
| `content` | string | — | Inline content, exclusive with src. |
| `dest` | string | required/conditional | Target destination; parent must be prepared for content installation. |
| `mode` | string | preserve / umask | Quoted octal permissions; preserve an existing file when omitted. |
| `owner` | string | preserve / execution identity | Target owner name or numeric ID string; resolved before mutation. |
| `group` | string | preserve / execution identity | Target group name/ID; user uses this as a primary-group name. |
| `vars` | mapping | {} | Template-local variable definitions resolved against the task context. |

<a id="behavior"></a>

## Validation and idempotence

Exactly one of src/content, dest required. Role templates and source-relative paths are supported. force defaults true. vars adds template-local definitions. Unsupported Jinja filters fail; output is not re-rendered.

<a id="returns"></a>

## Return values

common result fields. Absent/conditional fields must be tested before access. All modules use the [common result model](../results.md).

<a id="examples"></a>

## Example

The example is a complete playbook. Supply inventory, required variables/files and prepared target resources. Account/package/service examples require an isolated privileged target; example.invalid URLs are placeholders. Do not execute these blocks blindly.

```yaml
- hosts: all
  gather_facts: false
  vars:
    application_name: example
  tasks:
    - template:
        content: 'application={{ application_name }}'
        dest: '{{ demo_root }}/config'
        mode: '0600'
```

<a id="verification"></a>

## Verification and related guides

Verify actual target state, then repeat an idempotent operation. Check predicts/skips and must leave business state unchanged; it does not make arbitrary commands idempotent. no_log hides presentation, not the in-memory register. See [support boundaries](../compatibility.md), [examples](../../../../examples/README.md) and [test layers](../../development/testing.md).

<!-- parameters: src,content,dest,mode,owner,group,vars -->
