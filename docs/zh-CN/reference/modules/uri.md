[English / 简体中文](../../../en/reference/modules/uri.md) · [Index / 目录](../../index.md)

# uri

<a id="purpose"></a>

## 用途与执行位置

从目标发送有界 HTTP 请求。

目标 worker。check: skip; diff: false; async: false.

<a id="parameters"></a>

## 参数

| 参数 | 类型 | 默认值 / 必填关系 | 含义 |
|---|---|---|---|
| `url` | string | required | HTTP(S) 目标 URL，无 file:// 或 curl 回退。 |
| `method` | string | GET | HTTP 方法，校验前统一大小写。 |
| `headers` | mapping[string,string] | {} | HTTP 请求头；涉及凭据时使用 no_log。 |
| `body` | any | — | 按 body_format 解释的请求体。 |
| `body_format` | string | raw | raw 字符串、json 值/合法 JSON 字符串、form-urlencoded 字符串映射。 |
| `status_code` | integer/list[integer] | 200 | 期望 HTTP 状态码，100–599。 |
| `timeout` | integer | module-specific | wait_for 默认 300、uri 默认 30 秒；任务 timeout 是独立关键词。 |
| `return_content` | boolean | false | 返回 UTF-8 内容；非预期状态也返回有界内容。 |
| `validate_certs` | boolean | true | 校验目标 TLS 证书链。 |
| `use_proxy` | boolean | true | 使用目标代理环境。 |
| `ca_path` | string | 目标信任根 | 额外 PEM CA 文件。 |
| `follow_redirects` | string | safe | none、safe（GET/HEAD）、all；最多 10 次跳转。 |
| `url_username` | string | — | HTTP Basic 用户名，user 为别名。 |
| `url_password` | string | — | HTTP Basic 密码，password 为别名。 |
| `user` | string | — | url_username 的 HTTP 用户名别名。 |
| `password` | string | 不改变 | user：crypt 哈希而非明文；uri：HTTP 密码别名。 |
| `force_basic_auth` | boolean | false | 提前发送 Basic 凭据，不等待认证挑战。 |

<a id="behavior"></a>

## 校验与幂等

url 必填；默认 GET、期望200、超时30秒、raw请求体、校验证书/使用代理、安全跳转。请求/响应上限8 MiB；return_content 要求 UTF-8。认证相关响应受遮蔽；check 跳过，不预测资源变化。

<a id="returns"></a>

## 返回值

status, url; content 请求时返回/unexpected; json 成功解析 JSON 时。访问可选字段前应检查存在性；所有模块共用[结果模型](../results.md)。

<a id="examples"></a>

## 示例

下例为完整 playbook，需提供 inventory、所需变量/文件以及预备目标资源。账号/软件包/服务场景要求隔离的提权目标，example.invalid URL 是占位地址，不应盲目执行。

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

## 验证与相关指南

检查实际目标状态，再重跑幂等操作；check 预测/跳过且不改变业务状态，不会令任意命令自动幂等。no_log 隐藏展示，不清空内存 register。参见[支持边界](../compatibility.md)、[示例](../../../../examples/README.zh-CN.md)与[测试分层](../../development/testing.md)。

<!-- parameters: url,method,headers,body,body_format,status_code,timeout,return_content,validate_certs,use_proxy,ca_path,follow_redirects,url_username,url_password,user,password,force_basic_auth -->
