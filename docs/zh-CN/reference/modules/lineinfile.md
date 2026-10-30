[English / 简体中文](../../../en/reference/modules/lineinfile.md) · [Index / 目录](../../index.md)

# lineinfile

<a id="purpose"></a>

## 用途与执行位置

确保或删除文本行并保留安全元数据。

目标 worker。check: prediction; diff: true; async: true.

<a id="parameters"></a>

## 参数

| 参数 | 类型 | 默认值 / 必填关系 | 含义 |
|---|---|---|---|
| `path` | string | 必填/按场景 | 目标路径；file 可改用 dest，但不能同时提供。 |
| `line` | string | present 时必填 | 期望的完整行；无 regexp 时也用于精确删除。 |
| `regexp` | string | — | Rust 正则表达式，不支持 Python 特有结构。 |
| `state` | string | present | 期望资源状态，允许值见下文。 |
| `backup` | boolean | false | 内容变化时创建受保护备份。 |
| `create` | boolean | false | 允许创建缺失的 lineinfile 目标。 |
| `insertafter` | string | EOF | 插入锚点（正则/EOF），采用最后匹配行。 |
| `insertbefore` | string | — | 替代插入锚点（正则/BOF），与 insertafter 互斥。 |
| `owner` | string | 保留已有值 / 执行身份 | 目标属主名称或数字 ID 字符串，修改前解析。 |
| `group` | string | 保留已有值 / 执行身份 | 目标属组名称/ID；user 中表示主组名称。 |
| `mode` | string | 保留已有值 / umask | 带引号的八进制权限；省略时保留已有文件权限。 |

<a id="behavior"></a>

## 校验与幂等

state 默认 present；present 要求 line，absent 要求 line 或 regexp。采用最后匹配，否则按锚点/EOF 插入。UTF-8 输入上限 64 MiB，backup 保留保护元数据，不支持 Python 正则回引用。

<a id="returns"></a>

## 返回值

backup_file 创建时返回; 公共字段。访问可选字段前应检查存在性；所有模块共用[结果模型](../results.md)。

<a id="examples"></a>

## 示例

下例为完整 playbook，需提供 inventory、所需变量/文件以及预备目标资源。账号/软件包/服务场景要求隔离的提权目标，example.invalid URL 是占位地址，不应盲目执行。

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - lineinfile:
        path: '{{ demo_root }}/config'
        line: enabled=true
        create: true
```

<a id="verification"></a>

## 验证与相关指南

检查实际目标状态，再重跑幂等操作；check 预测/跳过且不改变业务状态，不会令任意命令自动幂等。no_log 隐藏展示，不清空内存 register。参见[支持边界](../compatibility.md)、[示例](../../../../examples/README.zh-CN.md)与[测试分层](../../development/testing.md)。

<!-- parameters: path,line,regexp,state,backup,create,insertafter,insertbefore,owner,group,mode -->
