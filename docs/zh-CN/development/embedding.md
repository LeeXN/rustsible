[English / 简体中文](../../en/development/embedding.md) · [Index / 目录](../index.md)

# 编译期 Rust 接口

<a id="registration"></a>

## 执行前注册

共享 Arc 前调用 Registry::new(...).with_compiled(Components{...})，通过 ExecutionOptions.extensions 或 AdHocOptions.extensions 传入。真实接入见 [compiled_extensions](https://github.com/LeeXN/rustsible/blob/main/tests/compiled_extensions.rs)。这是 Rust 源码接口，不是稳定动态库 ABI。

<a id="interfaces"></a>

## 已有边界

action：校验后的控制端函数，不支持 async/task timeout。callback：仅主机/模块/outcome/changed/check 完成事件，无结果正文。connection：工厂及有界 worker Session。become：bootstrap 命令及独立 Zeroizing stdin 前缀，仍校验 UID。strategy：基于 linear/free 的合法主机排列，保留 serial/throttle/失败规则。可信函数须自行遵守执行约定。

<a id="runner"></a>

## 嵌入控制端使用可执行提供器

入口先处理 rustsible_native::rpc::handle_runner_mode()，再进行普通参数/配置处理，与内置控制端一致；exec runner 避免多线程控制端 fork 后分配内存。外部 inventory/lookup/filter 仍遵守相同的显式安装与协议校验。
