[English / 简体中文](../../../en/reference/modules/setup.md) · [Index / 目录](../../index.md)

# setup

<a id="purpose"></a>

## 用途与执行位置

在实际执行目标收集 facts。

目标 worker。check: readonly; diff: false; async: false.

<a id="parameters"></a>

## 参数

| 参数 | 类型 | 默认值 / 必填关系 | 含义 |
|---|---|---|---|
| `gather_subset` | string/list[string] | all | 支持的 subset 名称及 ! 排除，详见 facts 参考。 |
| `filter` | string/list[string] | 全部返回键 | 匹配返回 facts 顶层名称的 Shell 模式。 |
| `gather_timeout` | integer | 10 | 目标 facts 收集预算，1–604800 秒。 |

<a id="behavior"></a>

## 校验与幂等

支持命名 subset 和顶层过滤；!all 保留最小集，!all,!min 从空集开始。显式 setup 与自动收集合并方式一致，无 Python 或可执行本地 facts 收集器。

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
    - setup:
        gather_subset: [platform, distribution]
        filter: [ansible_architecture, ansible_distribution]
```

<a id="verification"></a>

## 验证与相关指南

检查实际目标状态，再重跑幂等操作；check 预测/跳过且不改变业务状态，不会令任意命令自动幂等。no_log 隐藏展示，不清空内存 register。参见[支持边界](../compatibility.md)、[示例](../../../../examples/README.zh-CN.md)与[测试分层](../../development/testing.md)。

<!-- parameters: gather_subset,filter,gather_timeout -->
