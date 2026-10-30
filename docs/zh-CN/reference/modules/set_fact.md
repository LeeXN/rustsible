[English / 简体中文](../../../en/reference/modules/set_fact.md) · [Index / 目录](../../index.md)

# set_fact

<a id="purpose"></a>

## 用途与执行位置

发布类型化的持久主机变量。

控制端。check: readonly; diff: false; async: false.

<a id="parameters"></a>

## 参数

| 参数 | 类型 | 默认值 / 必填关系 | 含义 |
|---|---|---|---|
| `cacheable` | boolean | false | 仅接受 false，不支持持久 facts 缓存。 |
| `<variable>` | any | required | 一个或多个合法变量名及其类型化值。 |

<a id="behavior"></a>

## 校验与幂等

至少提供一个变量，仅在本次运行跨 play 保留，不是磁盘缓存；拒绝 cacheable:true。check 下仍可发布内存变量。

<a id="returns"></a>

## 返回值

ansible_facts。访问可选字段前应检查存在性；所有模块共用[结果模型](../results.md)。

<a id="examples"></a>

## 示例

下例为完整 playbook，需提供 inventory、所需变量/文件以及预备目标资源。账号/软件包/服务场景要求隔离的提权目标，example.invalid URL 是占位地址，不应盲目执行。

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - set_fact:
        release_ready: true
```

<a id="verification"></a>

## 验证与相关指南

检查实际目标状态，再重跑幂等操作；check 预测/跳过且不改变业务状态，不会令任意命令自动幂等。no_log 隐藏展示，不清空内存 register。参见[支持边界](../compatibility.md)、[示例](../../../../examples/README.zh-CN.md)与[测试分层](../../development/testing.md)。

<!-- parameters: cacheable,<variable> -->
