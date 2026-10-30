[English / 简体中文](../../../zh-CN/reference/modules/get_url.md) · [Index / 目录](../../index.md)

# get_url

<a id="purpose"></a>

## Purpose and location

Stream an HTTP(S) download on the target.

Target worker. check: prediction; diff: true; async: true.

<a id="parameters"></a>

## Parameters

| Parameter | Type | Default / requirement | Meaning |
|---|---|---|---|
| `url` | string | required | HTTP(S) target URL; no file:// or automatic curl fallback. |
| `dest` | string | required/conditional | Target destination; parent must be prepared for content installation. |
| `owner` | string | preserve / execution identity | Target owner name or numeric ID string; resolved before mutation. |
| `group` | string | preserve / execution identity | Target group name/ID; user uses this as a primary-group name. |
| `mode` | string | preserve / umask | Quoted octal permissions; preserve an existing file when omitted. |
| `force` | boolean | false | Whether differing existing content may be replaced; see behavior. |
| `checksum` | string | — | Inline algorithm:hex-digest, validated before downloading. |

<a id="behavior"></a>

## Validation and idempotence

url/dest required. force defaults false; existing content is retained unless supplied checksum differs. Metadata still applies. Validate checksum/mode/identity before network and commit atomically. Check performs HEAD only when needed, no destination staging.

<a id="returns"></a>

## Return values

dest, checksum_src when checksum supplied. Absent/conditional fields must be tested before access. All modules use the [common result model](../results.md).

<a id="examples"></a>

## Example

The example is a complete playbook. Supply inventory, required variables/files and prepared target resources. Account/package/service examples require an isolated privileged target; example.invalid URLs are placeholders. Do not execute these blocks blindly. 

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - get_url:
        url: https://example.invalid/release.bin
        dest: '{{ demo_root }}/release.bin'
        checksum: 'sha256:0000000000000000000000000000000000000000000000000000000000000000'
```

<a id="verification"></a>

## Verification and related guides

Verify actual target state, then repeat an idempotent operation. Check predicts/skips and must leave business state unchanged; it does not make arbitrary commands idempotent. no_log hides presentation, not the in-memory register. See [support boundaries](../compatibility.md), [examples](../../../../examples/README.md) and [test layers](../../development/testing.md).

<!-- parameters: url,dest,owner,group,mode,force,checksum -->
