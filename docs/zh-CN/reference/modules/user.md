[English / 简体中文](../../../en/reference/modules/user.md) · [Index / 目录](../../index.md)

# user

<a id="purpose"></a>

## 用途与执行位置

使用目标工具管理 Linux 账号。

目标 worker。check: prediction; diff: false; async: true.

<a id="parameters"></a>

## 参数

| 参数 | 类型 | 默认值 / 必填关系 | 含义 |
|---|---|---|---|
| `name` | string | 必填/按场景 | 资源名称；具体必填关系见本页行为说明。 |
| `state` | string | present | 期望资源状态，允许值见下文。 |
| `uid` | integer | 已有值 / 系统分配 | 小于 u32::MAX 的非负目标 UID。 |
| `gid` | integer | 已有值 / 系统分配 | 非负目标 GID；user 中不能同时指定 gid 和 group。 |
| `group` | string | 保留已有值 / 执行身份 | 目标属组名称/ID；user 中表示主组名称。 |
| `groups` | string/list[string] | 不改变 | 补充组，逗号分隔字符串或列表。 |
| `append` | boolean | false | 追加补充组而非替换。 |
| `home` | string | 已有值 / 系统策略 | 账号家目录路径。 |
| `shell` | string | 已有值 / 系统策略 | 账号登录 Shell 路径。 |
| `comment` | string | 不改变 | 账号描述。 |
| `password` | string | 不改变 | user：crypt 哈希而非明文；uri：HTTP 密码别名。 |
| `create_home` | boolean | true | 创建或修复家目录；修复需 mkhomedir_helper。 |
| `system` | boolean | false | 创建系统账号/组，不将已有资源转换为系统资源。 |
| `remove` | boolean | false | 删除账号时移除家目录数据。 |

<a id="behavior"></a>

## 校验与幂等

name 必填；state 为 present/absent，默认 present；gid/group 互斥。groups 默认替换补充组，append:true 改为追加。password 是 crypt 哈希；家目录修复需要 mkhomedir_helper。通常需 root/提权，仅在隔离主机操作。

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
    - user:
        name: example_app
        state: present
        system: true
```

<a id="verification"></a>

## 验证与相关指南

检查实际目标状态，再重跑幂等操作；check 预测/跳过且不改变业务状态，不会令任意命令自动幂等。no_log 隐藏展示，不清空内存 register。参见[支持边界](../compatibility.md)、[示例](../../../../examples/README.zh-CN.md)与[测试分层](../../development/testing.md)。

<!-- parameters: name,state,uid,gid,group,groups,append,home,shell,comment,password,create_home,system,remove -->
