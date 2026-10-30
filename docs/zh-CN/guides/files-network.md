[English / 简体中文](../../en/guides/files-network.md) · [Index / 目录](../index.md)

# 文件、下载与 HTTP

<a id="purpose"></a>

## 开始之前

copy/template/get_url/lineinfile 先暂存内容，再原子替换普通文件；保留未指定的权限/属主/属组及已有保护元数据，无法安全替换时失败，不截断原文件。八进制权限使用引号。

<a id="usage"></a>

## 用法

```bash
rustsible ad-hoc web -i inventory.yml -m stat -a 'path=/etc/passwd checksum_algorithm=sha256'
```

<a id="behavior"></a>

## 行为与边界

get_url 在目标下载并流式处理大文件，force:false 保留已有内容，除非提供的 checksum 不匹配。uri 用于有界 HTTP 请求/响应，不用于大文件下载；TLS 与代理使用目标设置。wait_for 支持 TCP、文件存在/消失和文本匹配；uri/wait_for 在 check 下跳过。git 需要目标 Git 工具；unarchive 原生解码 tar/gzip/ZIP 并拒绝越界路径和链接。

<a id="reference"></a>

## 相关参考

参见[模块索引](../reference/modules.md)、[配置](../reference/configuration.md)、[关键词](../reference/keywords.md)和[结果](../reference/results.md)。
