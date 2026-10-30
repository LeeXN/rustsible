[English / 简体中文](../../../en/reference/modules/git.md) · [Index / 目录](../../index.md)

# git

<a id="purpose"></a>

## 用途与执行位置

按目标 revision 克隆或更新检出。

目标 worker。check: prediction; diff: true; async: false.

<a id="parameters"></a>

## 参数

| 参数 | 类型 | 默认值 / 必填关系 | 含义 |
|---|---|---|---|
| `repo` | string | required | 目标 Git 仓库 URL/路径，拒绝以 - 开始的选项。 |
| `dest` | string | 必填/按场景 | 目标路径；内容安装前应准备父目录。 |
| `version` | string | HEAD | 在目标解析的提交、标签或分支。 |
| `update` | boolean | true | 抓取并更新已有仓库；false 保留当前检出。 |

<a id="behavior"></a>

## 校验与幂等

repo/dest 必填；默认 version HEAD、update:true。解析分支/标签/提交，建议固定提交以便复现。拒绝冲突的脏工作树；check 仅查询，缺 Git 时直接失败，不自动安装。

<a id="returns"></a>

## 返回值

before, after。访问可选字段前应检查存在性；所有模块共用[结果模型](../results.md)。

<a id="examples"></a>

## 示例

下例为完整 playbook，需提供 inventory、所需变量/文件以及预备目标资源。账号/软件包/服务场景要求隔离的提权目标，example.invalid URL 是占位地址，不应盲目执行。

```yaml
- hosts: all
  gather_facts: false
  tasks:
    - git:
        repo: https://example.invalid/app.git
        dest: '{{ demo_root }}/source'
        version: reviewed-commit
```

<a id="verification"></a>

## 验证与相关指南

检查实际目标状态，再重跑幂等操作；check 预测/跳过且不改变业务状态，不会令任意命令自动幂等。no_log 隐藏展示，不清空内存 register。参见[支持边界](../compatibility.md)、[示例](../../../../examples/README.zh-CN.md)与[测试分层](../../development/testing.md)。

<!-- parameters: repo,dest,version,update -->
