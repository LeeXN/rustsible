[English / 简体中文](../../../zh-CN/reference/modules/git.md) · [Index / 目录](../../index.md)

# git

<a id="purpose"></a>

## Purpose and location

Clone or update a checkout at a target revision.

Target worker. check: prediction; diff: true; async: false.

<a id="parameters"></a>

## Parameters

| Parameter | Type | Default / requirement | Meaning |
|---|---|---|---|
| `repo` | string | required | Target Git repository URL/path; options beginning with - are rejected. |
| `dest` | string | required/conditional | Target destination; parent must be prepared for content installation. |
| `version` | string | HEAD | Commit, tag or branch to resolve on target. |
| `update` | boolean | true | Fetch/update an existing repository; false keeps its current checkout. |

<a id="behavior"></a>

## Validation and idempotence

repo/dest required; version HEAD, update:true. Resolve branch/tag/commit; prefer pinned commit for repeatability. Reject dirty conflicting worktrees. Check uses readonly queries, missing Git fails without installation.

<a id="returns"></a>

## Return values

before, after. Absent/conditional fields must be tested before access. All modules use the [common result model](../results.md).

<a id="examples"></a>

## Example

The example is a complete playbook. Supply inventory, required variables/files and prepared target resources. Account/package/service examples require an isolated privileged target; example.invalid URLs are placeholders. Do not execute these blocks blindly. 

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - git:
        repo: https://example.invalid/app.git
        dest: '{{ demo_root }}/source'
        version: reviewed-commit
```

<a id="verification"></a>

## Verification and related guides

Verify actual target state, then repeat an idempotent operation. Check predicts/skips and must leave business state unchanged; it does not make arbitrary commands idempotent. no_log hides presentation, not the in-memory register. See [support boundaries](../compatibility.md), [examples](../../../../examples/README.md) and [test layers](../../development/testing.md).

<!-- parameters: repo,dest,version,update -->
