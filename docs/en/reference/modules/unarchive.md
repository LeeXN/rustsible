[English / 简体中文](../../../zh-CN/reference/modules/unarchive.md) · [Index / 目录](../../index.md)

# unarchive

<a id="purpose"></a>

## Purpose and location

Preflight and install tar, tar.gz or ZIP entries natively.

Target worker. check: prediction; diff: true; async: false.

<a id="parameters"></a>

## Parameters

| Parameter | Type | Default / requirement | Meaning |
|---|---|---|---|
| `src` | string | — | Source location; interpretation depends on the module. |
| `dest` | string | required/conditional | Target destination; parent must be prepared for content installation. |
| `remote_src` | boolean | false | Read archive on target when true; otherwise stream from controller. |
| `owner` | string | preserve / execution identity | Target owner name or numeric ID string; resolved before mutation. |
| `group` | string | preserve / execution identity | Target group name/ID; user uses this as a primary-group name. |
| `mode` | string | preserve / umask | Quoted octal permissions; preserve an existing file when omitted. |
| `creates` | string | — | Skip when this target path/glob already exists; unarchive uses an existence guard. |

<a id="behavior"></a>

## Validation and idempotence

src/dest required, dest absolute; remote_src:false streams controller archive, true requires absolute target src. Destination directory must exist. Reject links/devices/traversal/duplicates and decode all entries before writes. Limit8 GiB/100000 entries. Preserve unspecified existing metadata.

<a id="returns"></a>

## Return values

entries; common fields. Absent/conditional fields must be tested before access. All modules use the [common result model](../results.md).

<a id="examples"></a>

## Example

The example is a complete playbook. Supply inventory, required variables/files and prepared target resources. Account/package/service examples require an isolated privileged target; example.invalid URLs are placeholders. Do not execute these blocks blindly.

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - unarchive:
        src: release.tar.gz
        dest: '{{ demo_root }}/release'
        remote_src: false
```

<a id="verification"></a>

## Verification and related guides

Verify actual target state, then repeat an idempotent operation. Check predicts/skips and must leave business state unchanged; it does not make arbitrary commands idempotent. no_log hides presentation, not the in-memory register. See [support boundaries](../compatibility.md), [examples](../../../../examples/README.md) and [test layers](../../development/testing.md).

<!-- parameters: src,dest,remote_src,owner,group,mode,creates -->
