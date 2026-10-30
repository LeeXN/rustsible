[English / 简体中文](../../../en/reference/modules/group.md) · [Index / 目录](../../index.md)

# group

<a id="purpose"></a>

## 用途与执行位置

通过幂等查询管理 Linux 组。

目标 worker。check: prediction; diff: false; async: false.

<a id="parameters"></a>

## 参数

| 参数 | 类型 | 默认值 / 必填关系 | 含义 |
|---|---|---|---|
| `name` | string | 必填/按场景 | 资源名称；具体必填关系见本页行为说明。 |
| `gid` | integer | 已有值 / 系统分配 | 非负目标 GID；user 中不能同时指定 gid 和 group。 |
| `state` | string | present | 期望资源状态，允许值见下文。 |
| `system` | boolean | false | 创建系统账号/组，不将已有资源转换为系统资源。 |

<a id="behavior"></a>

## 校验与幂等

name 必填；state present/absent 默认 present；system 默认 false，仅在创建时生效。拒绝冲突 GID/名称变更，需要 groupadd/groupmod/groupdel。

<a id="returns"></a>

## 返回值

name, state, gid 可用时返回。访问可选字段前应检查存在性；所有模块共用[结果模型](../results.md)。

<a id="examples"></a>

## 示例

下例为完整 playbook，需提供 inventory、所需变量/文件以及预备目标资源。账号/软件包/服务场景要求隔离的提权目标，example.invalid URL 是占位地址，不应盲目执行。

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - group:
        name: example_app
        state: present
        system: true
```

<a id="verification"></a>

## 验证与相关指南

检查实际目标状态，再重跑幂等操作；check 预测/跳过且不改变业务状态，不会令任意命令自动幂等。no_log 隐藏展示，不清空内存 register。参见[支持边界](../compatibility.md)、[示例](../../../../examples/README.zh-CN.md)与[测试分层](../../development/testing.md)。

<!-- parameters: name,gid,state,system -->
