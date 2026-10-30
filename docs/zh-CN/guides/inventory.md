[English / 简体中文](../../en/guides/inventory.md) · [Index / 目录](../index.md)

# 主机清单与主机选择

<a id="purpose"></a>

## 开始之前

INI、YAML 支持主机、组和子组。本地示例明确设置 ansible_connection=local。all、glob、并集、交集（&）、排除（!）用于选择主机，--limit 进一步限制结果；hostvars 仍可包含当前 play 之外的主机。

<a id="usage"></a>

## 用法

```bash
rustsible playbook examples/quickstart/playbook.yml -i examples/quickstart/inventory.yml --list-hosts
```

<a id="behavior"></a>

## 行为与边界

inventory 名称是逻辑身份，不一定是 DNS 名。ansible_host 指定实际地址。group_vars/host_vars 先从清单旁加载，再从 playbook 旁加载；主机变量高于组变量。组环和非法连接值会报错。

<a id="reference"></a>

## 相关参考

参见[模块索引](../reference/modules.md)、[配置](../reference/configuration.md)、[关键词](../reference/keywords.md)和[结果](../reference/results.md)。
