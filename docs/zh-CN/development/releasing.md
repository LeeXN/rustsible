[English / 简体中文](../../en/development/releasing.md) · [Index / 目录](../index.md)

# 构建与验证离线包

<a id="artifacts"></a>

## 固定源码身份

控制端及 x86_64/aarch64 worker 使用同一源码和相同 RUSTSIBLE_BUILD_ID，便携包使用静态 musl target。打包需要 Cargo、tar、校验工具，解包运行不需要；交叉编译不能代替原生架构验收。

<a id="package"></a>

## 包布局与校验

```bash
bash scripts/package.sh CONTROLLER OUTPUT_DIRECTORY X86_64_WORKER AARCH64_WORKER
cd OUTPUT_DIRECTORY
sha256sum -c SHA256SUMS
./rustsible build-info
```

<a id="contents"></a>

## 交付内容

交付 rustsible、workers/manifest.json 及双 worker、RELEASE.json、SHA256SUMS、双 README、docs/en 与 docs/zh-CN、精选示例和 LICENSE。除校验清单自身外，递归覆盖所有交付文件。源码链接在包内转换为固定提交的 GitHub 链接，文档/示例链接保持本地可读；不交付测试凭据与验收程序。SDK 开发仍需源码和依赖。

<a id="acceptance"></a>

## 最终验收

从解包产物启动控制端，验证断网最小运行、双原生架构与 libc 环境、三机部署、二次幂等、check 快照、Vault、集合安装、handler、健康、逐字节回滚、异步和清理。release-target-evidence-ARCH 保存源码/构建身份、目标 UID/GID/mode/SHA 与零残留计数；下载后再次独立核对。CI 整体通过不代表覆盖未声明的后端组合。
