[English / 简体中文](../../en/guides/tasks.md) · [Index / 目录](../index.md)

# 任务控制与选择

<a id="purpose"></a>

## 开始之前

when 决定是否执行；changed_when/failed_when 覆盖结果条件，条件列表按 AND 合并。until 每次尝试更新 register，retries 表示额外重试次数，delay 默认 5 秒。timeout 限制每次目标执行并终止后代进程；uncertain/unreachable 会停止重试。

<a id="usage"></a>

## 用法

```bash
rustsible playbook examples/selection/playbook.yml -i examples/quickstart/inventory.yml --tags deploy --list-tasks
```

<a id="behavior"></a>

## 行为与边界

loop/with_items 接受列表或类型化表达式，loop_control 支持 loop_var、index_var；循环 register 保留逐项结果。play/task environment 仅作用于该请求。静态 tags 继承；动态 include 的 tags 选择 include 自身，apply 向子任务传播设置。never 任务须显式选中；已通知的 handler 不按任务 tags 过滤。

<a id="reference"></a>

## 相关参考

参见[模块索引](../reference/modules.md)、[配置](../reference/configuration.md)、[关键词](../reference/keywords.md)和[结果](../reference/results.md)。
