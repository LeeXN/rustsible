[English / 简体中文](../../../zh-CN/reference/modules/uri.md) · [Index / 目录](../../index.md)

# uri

<a id="purpose"></a>

## Purpose and location

Send a bounded HTTP request from the target.

Target worker. check: skip; diff: false; async: false.

<a id="parameters"></a>

## Parameters

| Parameter | Type | Default / requirement | Meaning |
|---|---|---|---|
| `url` | string | required | HTTP(S) target URL; no file:// or automatic curl fallback. |
| `method` | string | GET | HTTP method, case-normalized before validation. |
| `headers` | mapping[string,string] | {} | Explicit HTTP request headers; credentials require no_log. |
| `body` | any | — | Payload interpreted according to body_format. |
| `body_format` | string | raw | raw string, json value/valid JSON string, or form-urlencoded string mapping. |
| `status_code` | integer/list[integer] | 200 | Expected HTTP response status(es), 100–599. |
| `timeout` | integer | module-specific | wait_for:300, uri:30 seconds; task timeout is a separate keyword. |
| `return_content` | boolean | false | Return UTF-8 content; unexpected status also returns bounded content. |
| `validate_certs` | boolean | true | Validate the target TLS chain. |
| `use_proxy` | boolean | true | Use target proxy environment. |
| `ca_path` | string | target trust roots | Additional PEM CA file. |
| `follow_redirects` | string | safe | none, safe (GET/HEAD), or all; at most 10 redirects. |
| `url_username` | string | — | HTTP Basic username; user is an alias. |
| `url_password` | string | — | HTTP Basic password; password is an alias. |
| `user` | string | — | HTTP username alias for url_username. |
| `password` | string | unchanged | user: crypt hash, not plaintext. uri: HTTP password alias. |
| `force_basic_auth` | boolean | false | Send Basic credentials preemptively instead of waiting for a challenge. |

<a id="behavior"></a>

## Validation and idempotence

url required. Defaults GET, expected200, timeout30s, raw body, verify TLS/use proxy, safe redirects. Body/response limit8 MiB; return_content requires UTF-8. Authentication-related responses are censored. Requests skip in check, not a change prediction.

<a id="returns"></a>

## Return values

status, url; content when requested/unexpected; json for parsed JSON. Absent/conditional fields must be tested before access. All modules use the [common result model](../results.md).

<a id="examples"></a>

## Example

The example is a complete playbook. Supply inventory, required variables/files and prepared target resources. Account/package/service examples require an isolated privileged target; example.invalid URLs are placeholders. Do not execute these blocks blindly.

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - uri:
        url: http://127.0.0.1:8080/health
        status_code: 200
        return_content: true
```

<a id="verification"></a>

## Verification and related guides

Verify actual target state, then repeat an idempotent operation. Check predicts/skips and must leave business state unchanged; it does not make arbitrary commands idempotent. no_log hides presentation, not the in-memory register. See [support boundaries](../compatibility.md), [examples](../../../../examples/README.md) and [test layers](../../development/testing.md).

<!-- parameters: url,method,headers,body,body_format,status_code,timeout,return_content,validate_certs,use_proxy,ca_path,follow_redirects,url_username,url_password,user,password,force_basic_auth -->
