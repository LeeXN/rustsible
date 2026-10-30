[English / 简体中文](../../en/guides/connections.md) · [Index / 目录](../index.md)

# 连接、凭据与提权

<a id="purpose"></a>

## 开始之前

连接值从与任务参数相同的最终类型化上下文解析，可以覆盖关键词和默认设置。SSH 按有效主机/身份配置复用会话；reset_connection 或配置变化会新建连接租约。

<a id="usage"></a>

## 用法

```bash
rustsible ad-hoc web -i inventory.yml -m command -a 'id -un' --ask-pass --ask-become-pass -b
```

<a id="behavior"></a>

## 行为与边界

内置提权使用 sudo，相关变量是 ansible_become、ansible_become_user、ansible_become_password；密码经受保护输入传递，不进入进程参数。远端 worker 目录为 0700，请求文件为 0600。父目录 noexec 时须另设 rustsible_remote_tmp。SSH 严格检查主机密钥，非交互环境缺少凭据会报错。

<a id="reference"></a>

## 相关参考

参见[模块索引](../reference/modules.md)、[配置](../reference/configuration.md)、[关键词](../reference/keywords.md)和[结果](../reference/results.md)。
