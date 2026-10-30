[English / 简体中文](../../en/guides/troubleshooting.md) · [Index / 目录](../index.md)

# 按失败边界排障

<a id="purpose"></a>

## 开始之前

先检查 build-info、主机选择及语法/列表结果，核对控制端与 worker 的构建身份；确认有效连接值和真实执行身份，再判断模块缺陷。结果 uncertain 的修改任务不要直接重放。

<a id="usage"></a>

## 用法

```bash
rustsible build-info
rustsible playbook site.yml -i inventory.yml --syntax-check
rustsible playbook site.yml -i inventory.yml --list-hosts
```

<a id="behavior"></a>

## 行为与边界

主机密钥不符：核对指纹。临时目录 noexec/不安全：另选安全可执行的 rustsible_remote_tmp。未知参数/过滤器：查当前版本参考，不套用完整 Ansible 文档。权限错误：区分登录、提权、目的文件元数据。TLS 错误：配置目标 CA/代理，不直接关闭校验。缺少 SDK 程序：安装/配置集合并提供实际架构。no_log/Vault 遮蔽是预期行为，应改用无敏感信息的夹具复现，而不是打印秘密。

<a id="reference"></a>

## 相关参考

参见[模块索引](../reference/modules.md)、[配置](../reference/configuration.md)、[关键词](../reference/keywords.md)和[结果](../reference/results.md)。
