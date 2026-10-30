[English / 简体中文](README.md)

# 原生集合示例

本集合包含只读 probe 模块、inventory、JSON record lookup、slug filter、角色和 playbook。可执行程序源码为 crates/native-example；崩溃与写入夹具不交付。构建步骤在有 Rust 源码和依赖的环境进行，离线运行使用配套原生集合包。

先在每种对应架构的 Linux 构建机上执行以下命令，并将两份程序汇集到下文路径；需要对应 musl 链接工具。交叉编译须自行配置对应链接器，不能将编译成功当真实运行。

```bash
native_target="$(uname -m)-unknown-linux-musl"
rustup target add "$native_target"
cargo build --locked -p rustsible-native-example --release --target "$native_target"
```

```bash
cp -R examples/native-collection target/example-collection
cargo run --locked -p rustsible-native-example --example collection_manifest -- target/example-collection x86_64=target/x86_64-unknown-linux-musl/release/rustsible-example-extension aarch64=target/aarch64-unknown-linux-musl/release/rustsible-example-extension
rustsible collection pack target/example-collection --output target/example.native.tar.gz
rustsible collection verify target/example.native.tar.gz
rustsible collection install target/example.native.tar.gz
rustsible playbook example.native.inspect -i inventory.yml
```

先为两种架构构建静态程序；单架构实验可只传对应架构。清单生成器不运行包内程序。inventory 接受 path 指向 YAML inventory 对应结构的 JSON 文件；lookup 用 lookup('example.native.record', path)，filter 用 example.native.slug，均不再次模板化输出。
