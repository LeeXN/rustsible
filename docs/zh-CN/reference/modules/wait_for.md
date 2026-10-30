[English / 简体中文](../../../en/reference/modules/wait_for.md) · [Index / 目录](../../index.md)

# wait_for

<a id="purpose"></a>

## 用途与执行位置

等待 TCP 或目标文件条件。

目标 worker。check: skip; diff: false; async: false.

<a id="parameters"></a>

## 参数

| 参数 | 类型 | 默认值 / 必填关系 | 含义 |
|---|---|---|---|
| `host` | string | 127.0.0.1 | TCP 主机名/地址，在目标端解析 DNS。 |
| `port` | integer | — | TCP 端口 1–65535，与 path 互斥。 |
| `path` | string | 必填/按场景 | 目标路径；file 可改用 dest，但不能同时提供。 |
| `state` | string | started | 期望资源状态，允许值见下文。 |
| `search_regex` | string | — | 文件内容正则；要求 path、UTF-8 输入，上限 8 MiB。 |
| `timeout` | integer | module-specific | wait_for 默认 300、uri 默认 30 秒；任务 timeout 是独立关键词。 |
| `connect_timeout` | integer | 5 | 每次连接超时 1–604000 秒。 |
| `delay` | integer | 0 | 初始等待 0–604000 秒，计入 timeout。 |
| `sleep` | integer | 1 | 探测间隔 0–604000 秒。 |

<a id="behavior"></a>

## 校验与幂等

path/port 互斥；都不提供时仅等待。state 默认 started，present/started 对应出现，absent/stopped 对应消失。timeout300、delay0、connect_timeout5、sleep1 秒；timeout:0 不探测。正则仅用于存在性匹配和有界 UTF-8 文件，check 跳过。

<a id="returns"></a>

## 返回值

elapsed。访问可选字段前应检查存在性；所有模块共用[结果模型](../results.md)。

<a id="examples"></a>

## 示例

下例为完整 playbook，需提供 inventory、所需变量/文件以及预备目标资源。账号/软件包/服务场景要求隔离的提权目标，example.invalid URL 是占位地址，不应盲目执行。

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

## 验证与相关指南

检查实际目标状态，再重跑幂等操作；check 预测/跳过且不改变业务状态，不会令任意命令自动幂等。no_log 隐藏展示，不清空内存 register。参见[支持边界](../compatibility.md)、[示例](../../../../examples/README.zh-CN.md)与[测试分层](../../development/testing.md)。

<!-- parameters: host,port,path,state,search_regex,timeout,connect_timeout,delay,sleep -->
