[English / 简体中文](../../../en/reference/modules/assert.md) · [Index / 目录](../../index.md)

# assert

<a id="purpose"></a>

## 用途与执行位置

在控制端求值严格布尔条件。

控制端。check: readonly; diff: false; async: false.

<a id="parameters"></a>

## 参数

| 参数 | 类型 | 默认值 / 必填关系 | 含义 |
|---|---|---|---|
| `that` | boolean/string/list | required | 布尔条件或表达式，列表按 AND 合并。 |
| `fail_msg` | string | Assertion failed | 失败消息，msg 为别名。 |
| `msg` | any/string | — | 消息；assert/fail 为字符串，debug 可格式化结构值。 |
| `success_msg` | string | All assertions passed | 成功消息。 |
| `quiet` | boolean | false | 隐藏正常成功断言文本。 |

<a id="behavior"></a>

## 校验与幂等

that 必填，可为布尔值、表达式或列表，列表按 AND 合并。fail_msg/msg 是互斥别名，quiet 隐藏正常成功消息；断言本身不建立连接。

<a id="returns"></a>

## 返回值

assertion 失败时返回, evaluated_to; 公共字段。访问可选字段前应检查存在性；所有模块共用[结果模型](../results.md)。

<a id="examples"></a>

## 示例

下例为完整 playbook，需提供 inventory、所需变量/文件以及预备目标资源。账号/软件包/服务场景要求隔离的提权目标，example.invalid URL 是占位地址，不应盲目执行。

```yaml
- hosts: all
  gather_facts: false
  vars:
    release_ready: true
  tasks:
    - assert:
        that:
          - release_ready is defined
          - release_ready
```

<a id="verification"></a>

## 验证与相关指南

检查实际目标状态，再重跑幂等操作；check 预测/跳过且不改变业务状态，不会令任意命令自动幂等。no_log 隐藏展示，不清空内存 register。参见[支持边界](../compatibility.md)、[示例](../../../../examples/README.zh-CN.md)与[测试分层](../../development/testing.md)。

<!-- parameters: that,fail_msg,msg,success_msg,quiet -->
