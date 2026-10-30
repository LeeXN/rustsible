[English / 简体中文](../../../en/reference/modules/stat.md) · [Index / 目录](../../index.md)

# stat

<a id="purpose"></a>

## 用途与执行位置

不跟随链接地检查目标元数据与普通文件摘要。

目标 worker。check: readonly; diff: false; async: false.

<a id="parameters"></a>

## 参数

| 参数 | 类型 | 默认值 / 必填关系 | 含义 |
|---|---|---|---|
| `path` | string | 必填/按场景 | 目标路径；file 可改用 dest，但不能同时提供。 |
| `checksum_algorithm` | string | sha1 | md5、sha1、sha224、sha256、sha384、sha512。 |

<a id="behavior"></a>

## 校验与幂等

path 必填，摘要算法默认 sha1；缺失路径返回 stat.exists:false，仅稳定可读的普通文件有 checksum。拒绝不支持的算法。

<a id="returns"></a>

## 返回值

stat: exists; when present isreg/isdir/islnk/mode/uid/gid/size/inode/dev/nlink/mtime/ctime; regular-file checksum。访问可选字段前应检查存在性；所有模块共用[结果模型](../results.md)。

<a id="examples"></a>

## 示例

下例为完整 playbook，需提供 inventory、所需变量/文件以及预备目标资源。账号/软件包/服务场景要求隔离的提权目标，example.invalid URL 是占位地址，不应盲目执行。

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - stat:
        path: /etc/passwd
        checksum_algorithm: sha256
```

<a id="verification"></a>

## 验证与相关指南

检查实际目标状态，再重跑幂等操作；check 预测/跳过且不改变业务状态，不会令任意命令自动幂等。no_log 隐藏展示，不清空内存 register。参见[支持边界](../compatibility.md)、[示例](../../../../examples/README.zh-CN.md)与[测试分层](../../development/testing.md)。

<!-- parameters: path,checksum_algorithm -->
