[English / 简体中文](../../../en/reference/modules/async_status.md) · [Index / 目录](../../index.md)

# async_status

<a id="purpose"></a>

## 用途与执行位置

查询或清理属于执行身份的任务。

目标 worker。check: prediction; diff: false; async: false.

<a id="parameters"></a>

## 参数

| 参数 | 类型 | 默认值 / 必填关系 | 含义 |
|---|---|---|---|
| `jid` | string | required | 不透明 ID：j 加 64 位小写十六进制，不能是路径。 |
| `mode` | string | status | status 或 cleanup；check 下预测清理。 |

<a id="behavior"></a>

## 校验与幂等

jid 必填；mode status/cleanup 默认 status。ansible_async_dir 通过连接变量选择目录。清理仅删除安全的已完成任务，不能猜 ID 查询其他身份；保留失败和 no_log 状态。

<a id="returns"></a>

## 返回值

ansible_job_id, started, finished; 已完成模块字段。访问可选字段前应检查存在性；所有模块共用[结果模型](../results.md)。

<a id="examples"></a>

## 示例

下例为完整 playbook，需提供 inventory、所需变量/文件以及预备目标资源。账号/软件包/服务场景要求隔离的提权目标，example.invalid URL 是占位地址，不应盲目执行。

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - async_status:
        jid: '{{ background.ansible_job_id }}'
        mode: status
```

<a id="verification"></a>

## 验证与相关指南

检查实际目标状态，再重跑幂等操作；check 预测/跳过且不改变业务状态，不会令任意命令自动幂等。no_log 隐藏展示，不清空内存 register。参见[支持边界](../compatibility.md)、[示例](../../../../examples/README.zh-CN.md)与[测试分层](../../development/testing.md)。

<!-- parameters: jid,mode -->
