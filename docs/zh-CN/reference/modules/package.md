[English / 简体中文](../../../en/reference/modules/package.md) · [Index / 目录](../../index.md)

# package

<a id="purpose"></a>

## 用途与执行位置

使用检测出的目标管理器管理具体软件包名称。

目标 worker。check: prediction; diff: false; async: true.

<a id="parameters"></a>

## 参数

| 参数 | 类型 | 默认值 / 必填关系 | 含义 |
|---|---|---|---|
| `name` | string/list[string] | 必填/按场景 | 资源名称；具体必填关系见本页行为说明。 |
| `state` | string | present | 期望资源状态，允许值见下文。 |
| `update_cache` | boolean | false | 查询候选版本前刷新缓存，check 下不刷新。 |

<a id="behavior"></a>

## 校验与幂等

name 字符串/列表必填；state present/absent/latest 默认 present，update_cache 默认 false，刷新先于候选查询；check 仅读已有缓存。Zypper 拒绝 latest，APT 支持 name=version；取决于实际工具。

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
    - package:
        name: [git, vim]
        state: present
        update_cache: true
```

<a id="verification"></a>

## 验证与相关指南

检查实际目标状态，再重跑幂等操作；check 预测/跳过且不改变业务状态，不会令任意命令自动幂等。no_log 隐藏展示，不清空内存 register。参见[支持边界](../compatibility.md)、[示例](../../../../examples/README.zh-CN.md)与[测试分层](../../development/testing.md)。

<!-- parameters: name,state,update_cache -->
