[English / 简体中文](../../../en/reference/modules/command.md) · [Index / 目录](../../index.md)

# command

<a id="purpose"></a>

## 用途与执行位置

不经 Shell 执行参数数组。

目标 worker。check: guarded prediction; diff: false; async: true.

<a id="parameters"></a>

## 参数

| 参数 | 类型 | 默认值 / 必填关系 | 含义 |
|---|---|---|---|
| `cmd` | string | — | 程序及参数；cmd/argv 二选一，仅 shell 解释 Shell 语法。 |
| `argv` | list[string] | — | command 可执行文件及独立参数；首项非空，shell 不接受。 |
| `chdir` | string | 执行工作目录 | 目标工作目录。 |
| `creates` | string | — | 目标路径/glob 已存在时跳过；unarchive 用作存在性保护。 |
| `removes` | string | — | 仅目标路径/glob 存在时执行，在 creates 后判断。 |
| `stdin` | string | — | 通过标准输入传递，不放入 argv。 |
| `stdin_add_newline` | boolean | true | 为提供的 stdin 添加末尾换行。 |
| `strip_empty_ends` | boolean | true | 移除 stdout/stderr 末尾 CR/LF；文本精确保真备份应设 false。 |
| `expand_argument_vars` | boolean | true | 展开目标环境变量，不求值 Shell 语法；未知变量保留原文。 |

<a id="behavior"></a>

## 校验与幂等

cmd/argv 二选一，先判断 creates 再判断 removes。无保护条件时 check 跳过，有条件时预测；成功命令默认 changed，可用 changed_when 覆盖。

<a id="returns"></a>

## 返回值

rc, stdout, stderr, stdout_lines, stderr_lines。访问可选字段前应检查存在性；所有模块共用[结果模型](../results.md)。

<a id="examples"></a>

## 示例

下例为完整 playbook，需提供 inventory、所需变量/文件以及预备目标资源。账号/软件包/服务场景要求隔离的提权目标，example.invalid URL 是占位地址，不应盲目执行。

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - command:
        cmd: /bin/echo hello
```

<a id="verification"></a>

## 验证与相关指南

检查实际目标状态，再重跑幂等操作；check 预测/跳过且不改变业务状态，不会令任意命令自动幂等。no_log 隐藏展示，不清空内存 register。参见[支持边界](../compatibility.md)、[示例](../../../../examples/README.zh-CN.md)与[测试分层](../../development/testing.md)。

<!-- parameters: cmd,argv,chdir,creates,removes,stdin,stdin_add_newline,strip_empty_ends,expand_argument_vars -->
