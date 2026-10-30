[English / 简体中文](../../../zh-CN/reference/modules/user.md) · [Index / 目录](../../index.md)

# user

<a id="purpose"></a>

## Purpose and location

Manage Linux accounts using target account tools.

Target worker. check: prediction; diff: false; async: true.

<a id="parameters"></a>

## Parameters

| Parameter | Type | Default / requirement | Meaning |
|---|---|---|---|
| `name` | string | required/conditional | Resource name; see module-specific requirements below. |
| `state` | string | present | Desired resource state; accepted states are listed below. |
| `uid` | integer | existing / OS allocation | Nonnegative target UID below u32::MAX. |
| `gid` | integer | existing / OS allocation | Nonnegative target GID; user cannot specify both gid and group. |
| `group` | string | preserve / execution identity | Target group name/ID; user uses this as a primary-group name. |
| `groups` | string/list[string] | unchanged | Supplementary groups, comma-separated or a list. |
| `append` | boolean | false | Append supplementary groups instead of replacing them. |
| `home` | string | existing / OS policy | Account home path. |
| `shell` | string | existing / OS policy | Account login shell path. |
| `comment` | string | unchanged | Account description. |
| `password` | string | unchanged | user: crypt hash, not plaintext. uri: HTTP password alias. |
| `create_home` | boolean | true | Create or repair the home directory; repair needs mkhomedir_helper. |
| `system` | boolean | false | Create a system account/group; does not convert an existing resource. |
| `remove` | boolean | false | Remove home data when deleting an account. |

<a id="behavior"></a>

## Validation and idempotence

name required; state present/absent (default present). gid/group are exclusive. groups replaces supplementary groups unless append:true. password is a crypt hash. Home repair needs mkhomedir_helper. root/become is normally required; use disposable hosts.

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
    - user:
        name: example_app
        state: present
        system: true
```

<a id="verification"></a>

## Verification and related guides

Verify actual target state, then repeat an idempotent operation. Check predicts/skips and must leave business state unchanged; it does not make arbitrary commands idempotent. no_log hides presentation, not the in-memory register. See [support boundaries](../compatibility.md), [examples](../../../../examples/README.md) and [test layers](../../development/testing.md).

<!-- parameters: name,state,uid,gid,group,groups,append,home,shell,comment,password,create_home,system,remove -->
