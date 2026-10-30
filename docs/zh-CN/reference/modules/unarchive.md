[English / 简体中文](../../../en/reference/modules/unarchive.md) · [Index / 目录](../../index.md)

# unarchive

<a id="purpose"></a>

## 用途与执行位置

原生预检并安装 tar、tar.gz、ZIP 条目。

目标 worker。check: prediction; diff: true; async: false.

<a id="parameters"></a>

## 参数

| 参数 | 类型 | 默认值 / 必填关系 | 含义 |
|---|---|---|---|
| `src` | string | — | 源位置，含义取决于模块。 |
| `dest` | string | 必填/按场景 | 目标路径；内容安装前应准备父目录。 |
| `remote_src` | boolean | false | true 时读取目标归档，否则从控制端流式传递。 |
| `owner` | string | 保留已有值 / 执行身份 | 目标属主名称或数字 ID 字符串，修改前解析。 |
| `group` | string | 保留已有值 / 执行身份 | 目标属组名称/ID；user 中表示主组名称。 |
| `mode` | string | 保留已有值 / umask | 带引号的八进制权限；省略时保留已有文件权限。 |
| `creates` | string | — | 目标路径/glob 已存在时跳过；unarchive 用作存在性保护。 |

<a id="behavior"></a>

## 校验与幂等

src/dest 必填，dest 为绝对路径；remote_src:false 流式传递控制端归档，true 要求绝对目标 src；目的目录须存在。写入前完整解码，拒绝链接、设备、越界、重复条目；上限8 GiB/100000条目，保留未指定的已有元数据。

<a id="returns"></a>

## 返回值

entries; 公共字段。访问可选字段前应检查存在性；所有模块共用[结果模型](../results.md)。

<a id="examples"></a>

## 示例

下例为完整 playbook，需提供 inventory、所需变量/文件以及预备目标资源。账号/软件包/服务场景要求隔离的提权目标，example.invalid URL 是占位地址，不应盲目执行。

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - unarchive:
        src: release.tar.gz
        dest: '{{ demo_root }}/release'
        remote_src: false
```

<a id="verification"></a>

## 验证与相关指南

检查实际目标状态，再重跑幂等操作；check 预测/跳过且不改变业务状态，不会令任意命令自动幂等。no_log 隐藏展示，不清空内存 register。参见[支持边界](../compatibility.md)、[示例](../../../../examples/README.zh-CN.md)与[测试分层](../../development/testing.md)。

<!-- parameters: src,dest,remote_src,owner,group,mode,creates -->
