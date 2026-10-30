[English / 简体中文](../../en/development/contributing.md) · [Index / 目录](../index.md)

# 贡献变更

<a id="workflow"></a>

## 从行为出发

编辑前阅读所有调用方、共享校验器和相关测试，在公共入口修复行为，不增加各前端补丁。Rust API 为0.1，可以明确调整；可执行扩展有显式 API/版本校验。秘密不得进入 argv/日志，不将私有部署当隐含测试目标。

<a id="checks"></a>

## 必需检查

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo build --workspace --all-features --locked
cargo test --workspace --all-targets --all-features --locked
cargo run --locked --example docs -- --check
cargo llvm-cov --workspace --all-targets --all-features --locked --fail-under-lines 80
cargo build --workspace --release --all-features --locked
cargo audit
```

<a id="docs"></a>

## 维护双语

每篇公开 Markdown 有对应语言页；同步更新章节 ID、可执行块、参数/默认值及示例引用。用 --write 生成 CLI/目录并提交两种结果，再 --check。不要自动翻译标识符或宣称未经验证的兼容性；许可证保留权威原文，AGENTS.md 不属于文档工具范围。
