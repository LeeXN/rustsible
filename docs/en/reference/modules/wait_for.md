[English / 简体中文](../../../zh-CN/reference/modules/wait_for.md) · [Index / 目录](../../index.md)

# wait_for

<a id="purpose"></a>

## Purpose and location

Wait for TCP or target file conditions.

Target worker. check: skip; diff: false; async: false.

<a id="parameters"></a>

## Parameters

| Parameter | Type | Default / requirement | Meaning |
|---|---|---|---|
| `host` | string | 127.0.0.1 | TCP target hostname/address; DNS resolves on the target. |
| `port` | integer | — | TCP port 1–65535, exclusive with path. |
| `path` | string | required/conditional | Target path; file accepts dest as an alternative, never both. |
| `state` | string | started | Desired resource state; accepted states are listed below. |
| `search_regex` | string | — | File-content regex; requires path and UTF-8 input, at most 8 MiB. |
| `timeout` | integer | module-specific | wait_for:300 (0–604000), uri:30 (1–604000) seconds; task timeout is a separate keyword. |
| `connect_timeout` | integer | 5 | Per-connect timeout, 1–604000 seconds. |
| `delay` | integer | 0 | Initial wait delay, 0–604000 seconds; counts toward timeout. |
| `sleep` | integer | 1 | Seconds between probes, 0–604000. |

<a id="behavior"></a>

## Validation and idempotence

path/port exclusive; neither means a delay. state defaults started; present/started versus absent/stopped. timeout300, delay0, connect_timeout5, sleep1 seconds. timeout:0 performs no probe. Regex applies only to presence and UTF-8 bounded files. Skips in check.

<a id="returns"></a>

## Return values

elapsed. Absent/conditional fields must be tested before access. All modules use the [common result model](../results.md).

<a id="examples"></a>

## Example

The example is a complete playbook. Supply inventory, required variables/files and prepared target resources. Account/package/service examples require an isolated privileged target; example.invalid URLs are placeholders. Do not execute these blocks blindly. 

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - wait_for:
        host: 127.0.0.1
        port: 8080
        state: started
        timeout: 30
```

<a id="verification"></a>

## Verification and related guides

Verify actual target state, then repeat an idempotent operation. Check predicts/skips and must leave business state unchanged; it does not make arbitrary commands idempotent. no_log hides presentation, not the in-memory register. See [support boundaries](../compatibility.md), [examples](../../../../examples/README.md) and [test layers](../../development/testing.md).

<!-- parameters: host,port,path,state,search_regex,timeout,connect_timeout,delay,sleep -->
