[English / 简体中文](../../en/reference/extension-protocol.md) · [Index / 目录](../index.md)

# 原生可执行扩展 API v1

<a id="transport"></a>

## 传输与信任

扩展是明确受信任的 ELF 程序，不是沙箱。stdin/stdout 使用大端 u32 长度 JSON 帧，stderr 为有界诊断，不是第二结果通道。Probe 声明 FQCN/kind；Hello 必须匹配 API、wire、schema、能力后，Call 才传参数；Reply 含 API 版本和 ModuleResult。扩展无需与控制端共享 Git BUILD_ID，但须匹配 API/wire/schema。

<a id="manifest"></a>

## 集合清单

rustsible-collection.json 包含 format1、namespace/name/version、extensions；每项含 definition（FQCN、kind、check/diff/async/mutates 能力、参数类型/required）及按 x86_64/aarch64 索引的相对路径/SHA256。不能替换保留内置命名空间；module 按真实目标选架构，inventory/lookup/filter 需要控制端架构。

<a id="providers"></a>

## 提供器约定

module 在目标 worker 执行。inventory 接收配置参数，返回 values.inventory（与 YAML inventory 对应的 JSON 结构），不是旧 _meta 脚本协议。lookup 接收 terms/keywords，filter 接收 value/arguments/keywords，均返回 values.value。非 module 提供器必须声明只读/check，不支持 diff/async；输出是普通类型化数据。

<a id="limits"></a>

## 边界与生命周期

请求/结果16 MiB、stderr64 KiB、ELF256 MiB。程序须属于 root/当前 UID，不能让组或其他用户写入，集合目录也须安全；每次执行重验摘要/ELF。修改请求后崩溃可为 uncertain，不自动重放；监督进程回收脱离会话后代，async 保留私有程序副本直至作业清理。参见[SDK指南](../development/native-extensions.md)。
