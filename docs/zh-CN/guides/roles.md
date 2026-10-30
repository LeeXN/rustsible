[English / 简体中文](../../en/guides/roles.md) · [Index / 目录](../index.md)

# 导入、角色与失败处置

<a id="purpose"></a>

## 开始之前

静态 import 在加载时展开，动态 include 仅选中后展开。相对文件依据任务/角色的真实来源定位。角色支持 tasks/defaults/vars/handlers/meta 依赖及指定 *_from 入口，并拒绝导入和依赖环。

<a id="usage"></a>

## 用法

```bash
rustsible playbook examples/roles/main.yml -i examples/quickstart/inventory.yml --list-tasks
```

<a id="behavior"></a>

## 行为与边界

block 的任务失败可进入 rescue，再执行 always；不可达不等同模块失败，不进入普通 rescue。ignore_errors 不隐藏不可达或结果不确定，ignore_unreachable 需明确设置。通知去重，handler 按定义顺序执行；pre_tasks/tasks/post_tasks 划分刷新阶段，force_handlers 和 meta 控制刷新与主机/play 生命周期。

<a id="reference"></a>

## 相关参考

参见[模块索引](../reference/modules.md)、[配置](../reference/configuration.md)、[关键词](../reference/keywords.md)和[结果](../reference/results.md)。
