[English / 简体中文](../../../en/reference/modules/get_url.md) · [Index / 目录](../../index.md)

# get_url

<a id="purpose"></a>

## 用途与执行位置

在目标流式下载 HTTP(S) 文件。

目标 worker。check: prediction; diff: true; async: true.

<a id="parameters"></a>

## 参数

| 参数 | 类型 | 默认值 / 必填关系 | 含义 |
|---|---|---|---|
| `url` | string | required | HTTP(S) 目标 URL，无 file:// 或 curl 回退。 |
| `dest` | string | 必填/按场景 | 目标路径；内容安装前应准备父目录。 |
| `owner` | string | 保留已有值 / 执行身份 | 目标属主名称或数字 ID 字符串，修改前解析。 |
| `group` | string | 保留已有值 / 执行身份 | 目标属组名称/ID；user 中表示主组名称。 |
| `mode` | string | 保留已有值 / umask | 带引号的八进制权限；省略时保留已有文件权限。 |
| `force` | boolean | false | 是否替换不同的已有内容，见本页行为。 |
| `checksum` | string | — | 内联 algorithm:hex-digest，下载前校验格式。 |

<a id="behavior"></a>

## 校验与幂等

url/dest 必填，force 默认 false；保留已有内容，除非给出的 checksum 不符；仍应用元数据。联网前校验摘要/权限/身份，再原子提交。check 仅按需执行 HEAD，不创建目的暂存文件。

<a id="returns"></a>

## 返回值

dest, checksum_src 指定 checksum 时返回。访问可选字段前应检查存在性；所有模块共用[结果模型](../results.md)。

<a id="examples"></a>

## 示例

下例为完整 playbook，需提供 inventory、所需变量/文件以及预备目标资源。账号/软件包/服务场景要求隔离的提权目标，example.invalid URL 是占位地址，不应盲目执行。

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - get_url:
        url: https://example.invalid/release.bin
        dest: '{{ demo_root }}/release.bin'
        checksum: 'sha256:0000000000000000000000000000000000000000000000000000000000000000'
```

<a id="verification"></a>

## 验证与相关指南

检查实际目标状态，再重跑幂等操作；check 预测/跳过且不改变业务状态，不会令任意命令自动幂等。no_log 隐藏展示，不清空内存 register。参见[支持边界](../compatibility.md)、[示例](../../../../examples/README.zh-CN.md)与[测试分层](../../development/testing.md)。

<!-- parameters: url,dest,owner,group,mode,force,checksum -->
