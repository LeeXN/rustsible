[English / 简体中文](../../../en/reference/modules/template.md) · [Index / 目录](../../index.md)

# template

<a id="purpose"></a>

## 用途与执行位置

在控制端渲染 MiniJinja 并原子安装。

控制端渲染、目标端安装。check: prediction; diff: true; async: false.

<a id="parameters"></a>

## 参数

| 参数 | 类型 | 默认值 / 必填关系 | 含义 |
|---|---|---|---|
| `src` | string | — | 源位置，含义取决于模块。 |
| `content` | string | — | 内联内容，与 src 互斥。 |
| `dest` | string | 必填/按场景 | 目标路径；内容安装前应准备父目录。 |
| `mode` | string | 保留已有值 / umask | 带引号的八进制权限；省略时保留已有文件权限。 |
| `owner` | string | 保留已有值 / 执行身份 | 目标属主名称或数字 ID 字符串，修改前解析。 |
| `group` | string | 保留已有值 / 执行身份 | 目标属组名称/ID；user 中表示主组名称。 |
| `vars` | mapping | {} | 基于任务上下文解析的模板局部变量定义。 |

<a id="behavior"></a>

## 校验与幂等

src/content 二选一，dest 必填。支持角色 templates 和来源相对路径，force 默认 true；vars 添加模板局部定义。不支持的过滤器报错，输出不会再次渲染。

<a id="returns"></a>

## 返回值

公共结果字段。访问可选字段前应检查存在性；所有模块共用[结果模型](../results.md)。

<a id="examples"></a>

## 示例

下例为完整 playbook，需提供 inventory、所需变量/文件以及预备目标资源。账号/软件包/服务场景要求隔离的提权目标，example.invalid URL 是占位地址，不应盲目执行。

```yaml
- hosts: all
  gather_facts: false
  vars:
    application_name: example
  tasks:
    - template:
        content: 'application={{ application_name }}'
        dest: '{{ demo_root }}/config'
        mode: '0600'
```

<a id="verification"></a>

## 验证与相关指南

检查实际目标状态，再重跑幂等操作；check 预测/跳过且不改变业务状态，不会令任意命令自动幂等。no_log 隐藏展示，不清空内存 register。参见[支持边界](../compatibility.md)、[示例](../../../../examples/README.zh-CN.md)与[测试分层](../../development/testing.md)。

<!-- parameters: src,content,dest,mode,owner,group,vars -->
