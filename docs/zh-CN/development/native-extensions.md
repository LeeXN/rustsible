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

<a id="minimal"></a>

## 最小自定义模块

下例是完整模块，声明严格参数并返回 exists 布尔值；权限错误不会被伪装成文件缺失。它作为 rustsible-native-example 的 minimal_module example 编译，两种语言引用相同源码。

```rust
use anyhow::Result;
use rustsible_protocol::{
    extension::{serve, Capabilities, Definition, Kind, Parameter, ParameterType},
    ModuleResult,
};
use std::collections::{BTreeMap, HashMap};
fn main() -> Result<()> {
    let definition = Definition {
        name: "example.native.exists".into(),
        kind: Kind::Module,
        capabilities: Capabilities {
            check: true,
            diff: false,
            asynchronous: false,
            mutates: false,
        },
        parameters: BTreeMap::from([(
            "path".into(),
            Parameter {
                kind: ParameterType::String,
                required: true,
            },
        )]),
    };
    serve(&[definition], |_, call| {
        let path = call.args["path"]
            .as_str()
            .expect("SDK validates the schema");
        let exists = match std::fs::metadata(path) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.into()),
        };
        Ok(ModuleResult {
            values: HashMap::from([("exists".into(), serde_yaml::Value::Bool(exists))]),
            ..Default::default()
        })
    })
}
```

<a id="build"></a>

## 构建与打包

在有源码和依赖的环境构建各架构静态程序；作者工具仅检查字节，不执行包内程序。角色/playbook 作为声明式内容打包，使用前显式安装。具体步骤见[集合示例](../../../examples/native-collection/README.zh-CN.md)。

<a id="contracts"></a>

## 实现约定

声明严格参数和真实能力，操作系统修改前遵守 Call.check，敏感结果遵守 no_log。错误结果不回显输入秘密，stdout 只传协议。不实现 Python 桥或执行仓库中自动发现的程序。测试未知参数/类型、握手失败时尚未传参、check 状态不变、二次幂等、崩溃/超时、异步隐私和真实目标架构，详见[协议](../reference/extension-protocol.md)。
