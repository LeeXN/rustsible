# 分阶段计划的实现状态

当前已实现并验证计划 **01–16**，以及文件安全补充修复。**M1、M2 已达成；完整的 22 项计划尚未完成。**

| 计划项 | 交付 | 证据 |
| --- | --- | --- |
| 01 运行状态与 facts | 已实现 | [#3](https://github.com/LeeXN/rustsible/pull/3)，跨 play、失败主机排除、setup、no_log 回归 |
| 02 文件与下载安全 | 已实现 | [#4](https://github.com/LeeXN/rustsible/pull/4)、[#6](https://github.com/LeeXN/rustsible/pull/6)，原 inode/字节/元数据、短读、写入失败、链接替换、ACL/xattr、set-id 回归 |
| 03 模板与来源路径 | 已实现 | [#5](https://github.com/LeeXN/rustsible/pull/5)，固定 Ansible 2.21.0 文件对照、其他 cwd 启动、输出不再执行 |
| 04 变量与有效连接 | 已实现并验证 | [#7](https://github.com/LeeXN/rustsible/pull/7)， 单份类型化变量；分层优先级；实际 local/SSH 配置；动态端口、三种身份、非法参数与循环检查 |
| 05 模块校验、分发与结果 | 已实现并验证 | [#8](https://github.com/LeeXN/rustsible/pull/8)，共享注册表与参数类型；预演能力校验；五类结果；保留退出码/输出/已知修改；本地与真实 SSH 中断不重放 |
| 06 Rust worker、协议与离线包 | 已实现并验证 | [#9](https://github.com/LeeXN/rustsible/pull/9)，setup/stat/command/shell 原生路径；双架构 glibc/musl SSH；私有身份目录、清单校验、取消与后代回收；双 worker 离线包解包、校验与断网运行 |
| 07 文件与下载原生迁移 | 已实现并验证 | [#10](https://github.com/LeeXN/rustsible/pull/10)， 删除 Python、GNU cp/远端文件 Shell 路径；流式内容协议；父进程持有目标端暂存目录；本地 glibc/musl SSH 的无 Python/curl、私有 CA、代理、20 MiB、capabilities、磁盘/输入字节/inode 不足与清理已通过；最小环境控制端完整执行已通过，[双架构 CI 40823389637](https://github.com/LeeXN/rustsible/actions/runs/40823389637) 七作业通过，覆盖率 84.05% |
| 08 其余系统模块原生迁移 | 已实现并验证 | [#11](https://github.com/LeeXN/rustsible/pull/11)，user/service/systemd/package 全部进 worker；共享 schema；独立任务监督进程；真实 systemd/APT、OpenRC/APK、SysV、本地/SSH/三身份、密码/组/home/缓存更新及版本升级通过；[双架构 CI 40830347366](https://github.com/LeeXN/rustsible/actions/runs/40830347366) 八作业通过，包含 x86_64 Rocky DNF、CentOS Yum、Arch Pacman、openSUSE Zypper 离线本地仓库及真实 Upstart PID 1；DNF 显式刷新缓存 |
| 09 连接与调度 | 已实现并验证 | [#12](https://github.com/LeeXN/rustsible/pull/12)，Playbook/ad-hoc 共用固定线程领取主机；任务屏障、结果顺序和 forks 上限；有效配置变化关闭旧连接；meta reset_connection。真实 SSH 12 次目标调用仅 4 次认证，包含 reset 与两次配置变化；[双架构 CI 40831259231](https://github.com/LeeXN/rustsible/actions/runs/40831259231) 八作业通过 |
| 10 任务执行语义 | 已实现并验证 | [#13](https://github.com/LeeXN/rustsible/pull/13)，changed_when/failed_when、until/retries/delay、timeout、environment、command 参数与安全预测；固定 Ansible 2.21 文件对照、84.09% 覆盖率；[双架构 CI 40833587972](https://github.com/LeeXN/rustsible/actions/runs/40833587972) 八作业通过，真实 SSH 三身份、超时终止 setsid 后代且只执行一次；协议 v3 |
| 11 变量、配置与常用 facts | 已实现并验证 | [#14](https://github.com/LeeXN/rustsible/pull/14)，group_vars/host_vars、vars_files、include_vars、set_fact、-e；hostvars/groups/路径上下文；配置、环境与 CLI 优先级；目标端发行版/主机/网络/时间 facts；Ansible 2.21 对照、本地 85.17% 覆盖率；[双架构 CI 40853781614](https://github.com/LeeXN/rustsible/actions/runs/40853781614) 八作业通过，glibc/musl SSH 三身份、check、配置/play/task 身份与属主、hosts 完整域名别名均有实证 |
| 12–13 复用、roles 与失败处置 | 已实现并验证 | [#15](https://github.com/LeeXN/rustsible/pull/15)，静态/动态导入、角色作用域与依赖、来源路径、pre/post/handlers；block/rescue/always、失败上下文、ignore_unreachable、listen/force_handlers、五种 meta；固定 Ansible 2.21 文件对照、本地覆盖率 86.36%；[双架构 CI 40880109243](https://github.com/LeeXN/rustsible/actions/runs/40880109243) 八作业通过，包括 glibc/musl SSH 三身份、父 block 身份与显式 false、预演无业务文件及临时程序清理 |
| 14 发布编排 | 已实现并验证 | [#16](https://github.com/LeeXN/rustsible/pull/16)，serial/vars_files/原始分区与失败分母、throttle/run_once/失败阈值；委派身份和逐项 facts、linear/free、共享公开角色与独立逻辑主机连接；16 个本地真实文件场景、86.94% 覆盖率；[双架构 CI 40908464598](https://github.com/LeeXN/rustsible/actions/runs/40908464598) 八作业通过，包括三台独立 SSH 目标的 glibc/musl、三身份/属主、并行委派、幂等、check、清理和固定 Ansible 2.21 对照；libz-sys 已升级 1.1.31 修复 RUSTSEC-2026-0418 |
| 15 异步执行 | 已实现并验证 | [#17](https://github.com/LeeXN/rustsible/pull/17)，原生后台监督、async/poll、ad-hoc -B/-P、async_status/finished；0700/0600 按身份隔离、原子结果、程序保留、poll 清理、查询沿用 no_log；86.93% 覆盖率；[双架构 CI 40924530647](https://github.com/LeeXN/rustsible/actions/runs/40924530647) 八作业通过，glibc/musl SSH 断连后继续、跨身份拒绝、原生文件、setsid 超时、noexec 与清理均有实证；协议 v4 |
| 16 assert/fail/group | 已实现并验证 | [#18](https://github.com/LeeXN/rustsible/pull/18)，共用严格布尔条件和统一结果；控制端断言/失败、no_log/quiet、rescue；原生组生命周期/GID/冲突检查；87.05% 覆盖率；[双架构 CI 40928251351](https://github.com/LeeXN/rustsible/actions/runs/40928251351) 八作业通过，glibc/musl SSH 三身份与 check 有实证 |
| 17 wait_for/uri | 实现并验收中 | 目标端 TCP/文件/内容就绪；共用 HTTP、TLS/私有 CA/代理、方法/请求体/状态码、认证遮蔽、8 MiB 边界和 until；本地及 x86_64 glibc/musl SSH 三身份通过，固定参考与最终双架构 CI 待完成 |
| 18–19 制品模块与 CLI | 未实现 | 第三阶段 |
| 20–22 Vault、扩展与发布验收 | 未实现 | 第四阶段 |

当前验证包括 fmt、Clippy、全量测试、超过 80% 的行覆盖率、release 构建；真实 x86_64 SSH 的 deploy/root/app、check 前后业务状态一致、幂等、facts/register、权限/属主和清理；固定 ansible-core 2.21.0 的模板及条件对照；非法参数修改前失败，实际中断任务只执行一次、阻止后续循环项/后续 play。普通 Cargo 测试与参考环境分开。依赖审计保留既有 unsafe-libyaml 停止维护警告。

已有独立原生 worker、版本化协议、清单生成器与双架构离线包脚本。全部内置目标模块均走原生 worker，不调用 Python；get_url 使用原生 Rustls HTTP。无 Python/curl 的 Debian/Alpine、真实 systemd/OpenRC/SysV、双架构与离线包验收通过。DNF/Yum 自身可能依赖操作系统 Python，不属于 Rustsible 的运行依赖。22 的最终发布组合验收仍需后续能力完成后执行。

已启用 ubuntu-24.04 / ubuntu-24.04-arm 的完整质量门禁与真实 SSH 作业，两种架构都已实际完成 glibc/musl 的原生执行、三身份与密码 sudo、check、noexec、中断及后代回收；musl 场景也在目标环境运行了静态控制端。双架构包包含相同构建的两个 worker，解包后在断网的无 Python/curl 容器中实际完成 setup/stat。不能用交叉编译或工作流配置本身代替运行结果。

后续仍按原计划完整推进。06 的执行链和离线包单独审查，便于先固化安全与取消边界；依赖紧密的后续项目可合成一个 PR：07–08（文件下载与其余模块原生迁移）、09–10（连接/调度/执行条件）、11–13（变量来源/roles/失败处理）、14–15（发布编排/异步）、16–19（常用模块/CLI）、20–22（Vault/原生扩展/完整发布验收）。这些分组不缩减原计划中的功能与验收范围。
