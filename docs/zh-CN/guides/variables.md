[English / 简体中文](../../en/guides/variables.md) · [Index / 目录](../index.md)

# 变量、模板与 facts

<a id="purpose"></a>

## 开始之前

区分变量定义与普通数据。定义可以引用其他变量并检测循环；命令输出、原生 inventory 输出和已渲染字符串不会再次编译。单表达式保留原生类型，条件必须得出布尔值。

<a id="usage"></a>

## 用法

```bash
rustsible ad-hoc all -i examples/quickstart/inventory.yml -m debug -a '{"msg":"{{ greeting }}"}' -e greeting=hello
```

<a id="behavior"></a>

## 行为与边界

facts 与 register 跨 play 保留，play/task 局部变量不泄漏。setup 在目标收集 facts，包括时间；ansible_facts 命名空间去掉 setup 结果的 ansible_ 前缀。失败主机保留错误状态，需显式清除。hostvars 在任务边界观察持久主机状态。使用 default([])、default(omit)；password_hash 使用明确稳定的 salt。具体顺序与字段见参考手册。

<a id="reference"></a>

## 相关参考

参见[模块索引](../reference/modules.md)、[配置](../reference/configuration.md)、[关键词](../reference/keywords.md)和[结果](../reference/results.md)。
