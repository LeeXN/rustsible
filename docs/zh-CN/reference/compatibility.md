[English / 简体中文](../../en/reference/compatibility.md) · [Index / 目录](../index.md)

# 支持范围与兼容性差异

<a id="platform"></a>

## 已验证平台

控制端/目标为 Linux x86_64、aarch64，验证 glibc/musl、原生 Rust worker 和离线包。APT/APK、systemd/OpenRC/SysV 有双架构验收；DNF/Yum/Pacman/Zypper、Upstart 的真实验收仅 x86_64，未将其 ARM 组合标为已验证。DNF/Yum 可依赖操作系统 Python。

<a id="reference"></a>

## Ansible 对照

已支持语义与固定 ansible-core 2.21.0 对照，不等于复刻整个生态。内置模块支持 FQCN 别名，systemd_service 为 systemd 别名，完整范围见[模块目录](modules.md)。不提供 Windows/macOS、常驻 Agent、Galaxy Python 模块/插件桥或稳定 Rust 动态库 ABI。

<a id="differences"></a>

## 明确差异

全局 --check 不能被 check_mode:false 取消；允许临时 worker 暂存，不允许业务修改或后台启动。条件必须为布尔值，普通数据不再编译。角色 argument_specs、任务级 collections、vars_prompt、module_defaults、file.recurse、递归 copy、copy.remote_src、regex_replace 等未注册过滤器不支持。uri/wait_for 在 check 下跳过，command 需保护条件才预测。语法/列表操作只在控制端进行，动态内容在运行时选中。

<a id="limits"></a>

## 运行边界

命令输出/请求帧16 MiB；URI 请求响应及 wait 文件匹配8 MiB；lineinfile 输入64 MiB；扩展 ELF256 MiB、stderr64 KiB；归档/集合8 GiB、100000条目。未知参数/组合在业务修改前失败。发布与目标证据见[验收状态](../development/validation-status.md)。
