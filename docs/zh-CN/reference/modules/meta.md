[English / 简体中文](../../../en/reference/modules/meta.md) · [Index / 目录](../../index.md)

# meta

<a id="purpose"></a>

## 用途与执行位置

控制 playbook 执行及 handler 阶段。

控制端。check: readonly; diff: false; async: false.

<a id="parameters"></a>

## 参数

| 参数 | 类型 | 默认值 / 必填关系 | 含义 |
|---|---|---|---|
| `action` | string | required | flush_handlers、clear_host_errors、reset_connection、end_host、end_play。 |

<a id="behavior"></a>

## 校验与幂等

flush_handlers 刷新通知；clear_host_errors 清除主机错误；reset_connection 释放租约；end_host/end_play 结束执行。ad-hoc 仅支持 reset_connection。

<a id="returns"></a>

## 返回值

公共结果字段。访问可选字段前应检查存在性；所有模块共用[结果模型](../results.md)。

<a id="examples"></a>

## 示例

下例为完整 playbook，需提供 inventory、所需变量/文件以及预备目标资源。账号/软件包/服务场景要求隔离的提权目标，example.invalid URL 是占位地址，不应盲目执行。

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - meta:
        action: flush_handlers
```

<a id="verification"></a>

## 验证与相关指南

检查实际目标状态，再重跑幂等操作；check 预测/跳过且不改变业务状态，不会令任意命令自动幂等。no_log 隐藏展示，不清空内存 register。参见[支持边界](../compatibility.md)、[示例](../../../../examples/README.zh-CN.md)与[测试分层](../../development/testing.md)。

<!-- parameters: action -->
