[English / 简体中文](../../../en/reference/modules/service.md) · [Index / 目录](../../index.md)

# service

<a id="purpose"></a>

## 用途与执行位置

通过检测出的服务工具管理运行与开机状态。

目标 worker。check: prediction; diff: false; async: true.

<a id="parameters"></a>

## 参数

| 参数 | 类型 | 默认值 / 必填关系 | 含义 |
|---|---|---|---|
| `name` | string | 必填/按场景 | 资源名称；具体必填关系见本页行为说明。 |
| `state` | string | 不改变 | 期望资源状态，允许值见下文。 |
| `enabled` | boolean | 不改变 | 期望开机启用状态，独立于当前运行状态。 |

<a id="behavior"></a>

## 校验与幂等

name 必填，至少提供 state 或 enabled；不强加默认运行状态。restarted/reloaded 每次主动执行。systemd/OpenRC/SysV/Upstart 能力不同，不支持的启用操作会报错。

<a id="returns"></a>

## 返回值

state/enabled 请求时返回。访问可选字段前应检查存在性；所有模块共用[结果模型](../results.md)。

<a id="examples"></a>

## 示例

下例为完整 playbook，需提供 inventory、所需变量/文件以及预备目标资源。账号/软件包/服务场景要求隔离的提权目标，example.invalid URL 是占位地址，不应盲目执行。

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - service:
        name: example-app
        state: started
        enabled: true
```

<a id="verification"></a>

## 验证与相关指南

检查实际目标状态，再重跑幂等操作；check 预测/跳过且不改变业务状态，不会令任意命令自动幂等。no_log 隐藏展示，不清空内存 register。参见[支持边界](../compatibility.md)、[示例](../../../../examples/README.zh-CN.md)与[测试分层](../../development/testing.md)。

<!-- parameters: name,state,enabled -->
