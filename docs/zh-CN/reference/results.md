[English / 简体中文](../../en/reference/results.md) · [Index / 目录](../index.md)

# 结果、注册与错误

<a id="fields"></a>

## 公共字段

changed、failed、skipped、unreachable、uncertain、msg 描述执行结果；命令提供时有 rc/stdout/stderr 及行数组，模块专有字段见各模块页。访问可选字段前检查存在性；失败命令保留 rc 和输出，已发生变化后失败仍保留 changed:true。

<a id="ownership"></a>

## 注册与归属

每次重试更新 register，循环保留逐项结果与 attempts。委派改变执行目标，但 register 仍属逻辑主机，delegate_facts 仅显式移动 facts。数据不会自动再模板化；failed_when/changed_when 使用当前结果，通知遵循最终 changed/failed 语义。

<a id="failure"></a>

## 失败边界

unreachable 表示连接失败；uncertain 表示修改请求可能执行，但无法确认结果，不能安全地自动重放。失败主机跨 play 保留排除状态直至清除；普通失败可进入 rescue，ignore_errors 不抹掉注册结果中的失败标记。

<a id="display"></a>

## 展示不等于数据存储

no_log 遮蔽展示，不清空内存 register；异步隐私在状态记录中保留。diff 大于64 KiB 或含二进制 NUL 的字符串会摘要化，整体 diff 也有限制；Vault 输入会保护敏感诊断/diff。源文件加密不意味着可以安全 debug 输出秘密。
