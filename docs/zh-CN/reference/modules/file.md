[English / 简体中文](../../../en/reference/modules/file.md) · [Index / 目录](../../index.md)

# file

<a id="purpose"></a>

## 用途与执行位置

管理原生文件、目录、链接与元数据。

目标 worker。check: prediction; diff: true; async: true.

<a id="parameters"></a>

## 参数

| 参数 | 类型 | 默认值 / 必填关系 | 含义 |
|---|---|---|---|
| `path` | string | 必填/按场景 | 目标路径；file 可改用 dest，但不能同时提供。 |
| `dest` | string | 必填/按场景 | 目标路径；内容安装前应准备父目录。 |
| `state` | string | file | 期望资源状态，允许值见下文。 |
| `src` | string | — | state:link 必需的符号链接目标。 |
| `mode` | string | 保留已有值 / umask | 带引号的八进制权限；省略时保留已有文件权限。 |
| `owner` | string | 保留已有值 / 执行身份 | 目标属主名称或数字 ID 字符串，修改前解析。 |
| `group` | string | 保留已有值 / 执行身份 | 目标属组名称/ID；user 中表示主组名称。 |

<a id="behavior"></a>

## 校验与幂等

path/dest 二选一。默认 state=file 不创建缺失内容；支持 directory/link/absent/touch/file，link 要求 src。支持创建缺失父目录，不支持递归修复全部元数据；touch 会主动更新时间。

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
    - file:
        path: '{{ demo_root }}/data'
        state: directory
        mode: '0700'
```

<a id="verification"></a>

## 验证与相关指南

检查实际目标状态，再重跑幂等操作；check 预测/跳过且不改变业务状态，不会令任意命令自动幂等。no_log 隐藏展示，不清空内存 register。参见[支持边界](../compatibility.md)、[示例](../../../../examples/README.zh-CN.md)与[测试分层](../../development/testing.md)。

<!-- parameters: path,dest,state,src,mode,owner,group -->
