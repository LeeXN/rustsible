[English / 简体中文](../../../en/reference/modules/include_vars.md) · [Index / 目录](../../index.md)

# include_vars

<a id="purpose"></a>

## 用途与执行位置

在任务执行时加载控制端变量文件/目录。

控制端。check: readonly; diff: false; async: false.

<a id="parameters"></a>

## 参数

| 参数 | 类型 | 默认值 / 必填关系 | 含义 |
|---|---|---|---|
| `file` | string | — | 控制端变量文件，与 dir 互斥。 |
| `dir` | string | — | 递归按字典序读取的控制端变量目录。 |
| `name` | string | — | 加载变量的可选命名空间。 |

<a id="behavior"></a>

## 校验与幂等

file/dir 二选一；name 创建命名空间。文件路径依据任务来源，目录递归按字典序合并；Vault 在内存解密。

<a id="returns"></a>

## 返回值

ansible_facts, ansible_included_var_files。访问可选字段前应检查存在性；所有模块共用[结果模型](../results.md)。

<a id="examples"></a>

## 示例

下例为完整 playbook，需提供 inventory、所需变量/文件以及预备目标资源。账号/软件包/服务场景要求隔离的提权目标，example.invalid URL 是占位地址，不应盲目执行。

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - include_vars:
        file: vars.yml
```

<a id="verification"></a>

## 验证与相关指南

检查实际目标状态，再重跑幂等操作；check 预测/跳过且不改变业务状态，不会令任意命令自动幂等。no_log 隐藏展示，不清空内存 register。参见[支持边界](../compatibility.md)、[示例](../../../../examples/README.zh-CN.md)与[测试分层](../../development/testing.md)。

<!-- parameters: file,dir,name -->
