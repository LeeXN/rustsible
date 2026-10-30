[English / 简体中文](../../en/development/native-extensions.md) · [Index / 目录](../index.md)

# 编写原生扩展

<a id="example"></a>

## 从随附 SDK 示例开始

完整示例源码位于 [crates/native-example](https://github.com/LeeXN/rustsible/tree/main/crates/native-example)，一个程序提供只读 probe、inventory、JSON record lookup、slug filter。崩溃/写入夹具仅用于验收，不属于发布集合。

<a id="serve"></a>

## SDK 入口

```rust
fn main() -> anyhow::Result<()> {
    rustsible_protocol::extension::serve(
        &rustsible_native_example::catalog(),
        rustsible_native_example::handle,
    )
}
```

<a id="build"></a>

## 构建与打包

在有源码和依赖的环境构建各架构静态程序；作者工具仅检查字节，不执行包内程序。角色/playbook 作为声明式内容打包，使用前显式安装。具体步骤见[集合示例](../../../examples/native-collection/README.zh-CN.md)。

<a id="contracts"></a>

## 实现约定

声明严格参数和真实能力，操作系统修改前遵守 Call.check，敏感结果遵守 no_log。错误结果不回显输入秘密，stdout 只传协议。不实现 Python 桥或执行仓库中自动发现的程序。测试未知参数/类型、握手失败时尚未传参、check 状态不变、二次幂等、崩溃/超时、异步隐私和真实目标架构，详见[协议](../reference/extension-protocol.md)。
