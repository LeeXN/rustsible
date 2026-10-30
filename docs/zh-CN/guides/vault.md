[English / 简体中文](../../en/guides/vault.md) · [Index / 目录](../index.md)

# 加密配置

<a id="purpose"></a>

## 开始之前

Vault 支持 AES256 1.1/1.2 文件、!vault 标量、vault ID、隐藏提示及普通不可执行密码文件。ID 引导尝试顺序，不限制其他正确密码解密。加载器在内存解密；明确的文件部署仍会写入请求的业务内容。

<a id="usage"></a>

## 用法

```bash
rustsible vault encrypt vars.yml --vault-id prod@prompt
rustsible vault view vars.yml --vault-id prod@prompt
rustsible vault rekey vars.yml --vault-id prod@prompt --new-vault-password-file /secure/new-password
```

<a id="behavior"></a>

## 行为与边界

使用 --vault-password-file、重复的 --vault-id ID@FILE、ID@prompt 或 --ask-vault-pass。CLI decrypt/view/output - 明确输出明文，须自行选择输出位置。密码脚本会被拒绝。使用 Vault 输入后，失败诊断与 diff 会受保护；主动打印敏感值的任务仍应设置 no_log。未访问的加密标量保持惰性，访问时需要正确密码。

<a id="reference"></a>

## 相关参考

参见[模块索引](../reference/modules.md)、[配置](../reference/configuration.md)、[关键词](../reference/keywords.md)和[结果](../reference/results.md)。
