[English / 简体中文](../../en/guides/scheduling.md) · [Index / 目录](../index.md)

# 滚动与委派执行

<a id="purpose"></a>

## 开始之前

linear 保留任务屏障，free 允许健康主机独立前进。forks 限制活跃主机控制线程，throttle 增加任务上限；serial 按数量、比例或列表对原始主机集分批，run_once 按 serial 批次执行。

<a id="usage"></a>

## 用法

```bash
rustsible playbook examples/rolling/playbook.yml -i examples/rolling/inventory -e demo_root=/tmp/reviewed-rollout --check
```

<a id="behavior"></a>

## 行为与边界

delegate_to 改变实际执行位置，不改变 register/通知的逻辑归属；delegate_facts 明确将 facts 归给被委派主机。any_errors_fatal 与严格的大于 max_fail_percentage 阈值停止后续工作。free 不接受 run_once、any_errors_fatal、max_fail_percentage 组合。fail_fast 是 Rustsible 扩展；实际运行使用新的、由操作者持有的 demo_root。

<a id="reference"></a>

## 相关参考

参见[模块索引](../reference/modules.md)、[配置](../reference/configuration.md)、[关键词](../reference/keywords.md)和[结果](../reference/results.md)。
