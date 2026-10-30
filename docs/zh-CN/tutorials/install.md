[English / 简体中文](../../en/tutorials/install.md) · [Index / 目录](../index.md)

# 安装与校验

<a id="requirements"></a>

## 选择正确产物

控制端与目标端支持 Linux x86_64、aarch64。控制端程序须匹配控制机；每个离线包包含两种架构的静态 worker。远端需要 SSH、POSIX sh、uname、id、stat、mktemp、head、wc、sha256sum、cut、chmod、rm；提权需要 sudo，各模块另有系统工具依赖。Rustsible 自身不要求 Python 或 curl。

<a id="verify"></a>

## 执行前解包校验

从同一 CI/发布产物取得匹配的压缩包，按需取得原生集合包。ARM 控制机将下例 x86_64 换成 aarch64。rustsible 与 workers/ 保持相邻；RELEASE.json 记录源码身份与程序摘要，build-info 不加载配置或 inventory。

```bash
tar -xzf x86_64.tar.gz
cd x86_64
sha256sum -c SHA256SUMS
./rustsible build-info
export PATH="$PWD:$PATH"
```

<a id="source"></a>

## 从源码构建

开发时先构建整个 workspace 再测试。普通主机构建仅生成本机 worker；双架构包需要同一源码的两种 worker。无 Git 元数据的源码包须设置 RUSTSIBLE_BUILD_ID。不要混用不同构建身份的控制端和 worker。

```bash
cargo build --workspace --all-features --locked
cargo run --locked -p rustsible-protocol --example worker_manifest -- target/debug/workers "$(uname -m)=target/debug/rustsible-worker"
target/debug/rustsible build-info
```

<a id="next"></a>

## 下一步

继续阅读[本地部署](local.md)，再进行 [SSH 部署](ssh.md)。离线包包含文档与示例；开发 SDK 还需要源码与 Cargo 依赖缓存。
