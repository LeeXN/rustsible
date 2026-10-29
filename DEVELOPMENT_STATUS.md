# 分阶段计划的实现状态

当前已实现并验证计划 **01–06**，以及文件安全补充修复。**完整的 22 项计划尚未完成，M1 尚未达成。**

| 计划项 | 交付 | 证据 |
| --- | --- | --- |
| 01 运行状态与 facts | 已实现 | [#3](https://github.com/LeeXN/rustsible/pull/3)，跨 play、失败主机排除、setup、no_log 回归 |
| 02 文件与下载安全 | 已实现 | [#4](https://github.com/LeeXN/rustsible/pull/4)、[#6](https://github.com/LeeXN/rustsible/pull/6)，原 inode/字节/元数据、短读、写入失败、链接替换、ACL/xattr、set-id 回归 |
| 03 模板与来源路径 | 已实现 | [#5](https://github.com/LeeXN/rustsible/pull/5)，固定 Ansible 2.21.0 文件对照、其他 cwd 启动、输出不再执行 |
| 04 变量与有效连接 | 已实现并验证 | [#7](https://github.com/LeeXN/rustsible/pull/7)， 单份类型化变量；分层优先级；实际 local/SSH 配置；动态端口、三种身份、非法参数与循环检查 |
| 05 模块校验、分发与结果 | 已实现并验证 | [#8](https://github.com/LeeXN/rustsible/pull/8)，共享注册表与参数类型；预演能力校验；五类结果；保留退出码/输出/已知修改；本地与真实 SSH 中断不重放 |
| 06 Rust worker、协议与离线包 | 已实现并验证 | [#9](https://github.com/LeeXN/rustsible/pull/9)，setup/stat/command/shell 原生路径；双架构 glibc/musl SSH；私有身份目录、清单校验、取消与后代回收；双 worker 离线包解包、校验与断网运行 |
| 07–08 文件下载与其余模块迁移 | 未实现 | get_url 仍需目标 Python；其余模块的目标端 Shell 路径待迁移 |
| 09–15 调度与部署流程 | 未实现 | 第二阶段 |
| 16–19 常用模块与 CLI | 未实现 | 第三阶段 |
| 20–22 Vault、扩展与发布验收 | 未实现 | 第四阶段 |

当前验证包括 fmt、Clippy、全量测试、超过 80% 的行覆盖率、release 构建；真实 x86_64 SSH 的 deploy/root/app、check 前后业务状态一致、幂等、facts/register、权限/属主和清理；固定 ansible-core 2.21.0 的模板及条件对照；非法参数修改前失败，实际中断任务只执行一次、阻止后续循环项/后续 play。普通 Cargo 测试与参考环境分开。依赖审计保留既有 unsafe-libyaml 停止维护警告。

已有独立原生 worker、版本化协议、清单生成器与双架构离线包脚本。setup/stat/command/shell 不再调用旧模块路径；get_url 仍依赖目标端 Python。M1 仍需 07–08；无 Python 的整个内置模块集、真实 systemd、最终发布组合验收尚未完成。06 的原生 x86_64/aarch64 运行证据与离线包校验见 [PR #9 的 CI 检查](https://github.com/LeeXN/rustsible/pull/9/checks)；该验收不代表尚未迁移的模块也已完成无 Python 验证。

已启用 ubuntu-24.04 / ubuntu-24.04-arm 的完整质量门禁与真实 SSH 作业，两种架构都已实际完成 glibc/musl 的原生执行、三身份与密码 sudo、check、noexec、中断及后代回收；musl 场景也在目标环境运行了静态控制端。双架构包包含相同构建的两个 worker，解包后在断网的无 Python/curl 容器中实际完成 setup/stat。不能用交叉编译或工作流配置本身代替运行结果。

后续仍按原计划完整推进。06 的执行链和离线包单独审查，便于先固化安全与取消边界；依赖紧密的后续项目可合成一个 PR：07–08（文件下载与其余模块原生迁移）、09–10（连接/调度/执行条件）、11–13（变量来源/roles/失败处理）、14–15（发布编排/异步）、16–19（常用模块/CLI）、20–22（Vault/原生扩展/完整发布验收）。这些分组不缩减原计划中的功能与验收范围。
