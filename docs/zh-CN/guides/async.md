[English / 简体中文](../../en/guides/async.md) · [Index / 目录](../index.md)

# 原生后台任务

<a id="purpose"></a>

## 开始之前

async 设置最长生命周期；poll>0 等待并清理完成状态，poll:0 返回不透明 job ID，供后续 async_status 查询与显式清理。已接受的后台任务不因 SSH 断开而停止，check 不启动后台任务。

<a id="usage"></a>

## 用法

```bash
rustsible ad-hoc web -i inventory.yml -m command -a 'sleep 10' -B 30 -P 0
```

<a id="behavior"></a>

## 行为与边界

状态属于执行身份，默认位于 ~/.rustsible_async，目录 0700、文件 0600；ansible_async_dir 可选私有位置。监督进程保留所需程序并在 TTL 后回收后代。私有结果在后续查询时仍遮蔽。模块须声明 async 能力，控制端文件上传不能直接后台化。

<a id="reference"></a>

## 相关参考

参见[模块索引](../reference/modules.md)、[配置](../reference/configuration.md)、[关键词](../reference/keywords.md)和[结果](../reference/results.md)。
