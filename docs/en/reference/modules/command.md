[English / 简体中文](../../../zh-CN/reference/modules/command.md) · [Index / 目录](../../index.md)

# command

<a id="purpose"></a>

## Purpose and location

Execute an argument vector without a shell.

Target worker. check: guarded prediction; diff: false; async: true.

<a id="parameters"></a>

## Parameters

| Parameter | Type | Default / requirement | Meaning |
|---|---|---|---|
| `cmd` | string | — | Program and arguments; exactly one of cmd/argv. Shell syntax only in shell. |
| `argv` | list[string] | — | Command executable and separate arguments; nonempty first item. Not accepted by shell. |
| `chdir` | string | execution cwd | Target working directory. |
| `creates` | string | — | Skip when this target path/glob already exists; unarchive uses an existence guard. |
| `removes` | string | — | Skip unless this target path/glob exists; evaluated after creates. |
| `stdin` | string | — | Input sent to the program without putting it in argv. |
| `stdin_add_newline` | boolean | true | Append a final newline to supplied stdin. |
| `strip_empty_ends` | boolean | true | Strip trailing CR/LF from stdout/stderr; set false for byte-exact text backups. |
| `expand_argument_vars` | boolean | true | Expand target environment variables without evaluating shell syntax; unknown names remain literal. |

<a id="behavior"></a>

## Validation and idempotence

Exactly one of cmd/argv. creates is checked before removes. Without a guard check skips; with a guard it predicts. A successful command is changed unless changed_when overrides it.

<a id="returns"></a>

## Return values

rc, stdout, stderr, stdout_lines, stderr_lines. Absent/conditional fields must be tested before access. All modules use the [common result model](../results.md).

<a id="examples"></a>

## Example

The example is a complete playbook. Supply inventory, required variables/files and prepared target resources. Account/package/service examples require an isolated privileged target; example.invalid URLs are placeholders. Do not execute these blocks blindly. 

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - command:
        cmd: /bin/echo hello
```

<a id="verification"></a>

## Verification and related guides

Verify actual target state, then repeat an idempotent operation. Check predicts/skips and must leave business state unchanged; it does not make arbitrary commands idempotent. no_log hides presentation, not the in-memory register. See [support boundaries](../compatibility.md), [examples](../../../../examples/README.md) and [test layers](../../development/testing.md).

<!-- parameters: cmd,argv,chdir,creates,removes,stdin,stdin_add_newline,strip_empty_ends,expand_argument_vars -->
