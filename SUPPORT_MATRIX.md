# 支持范围与验收边界

参考语义固定为 ansible-core 2.21.0。支持是下述明确子集，不是整个 Ansible/collections 生态。控制端和目标端为 Linux x86_64、aarch64；静态 musl 发布程序可在验证过的 glibc/musl 系统运行。SSH 目标需要 SSH 服务、POSIX shell 与基础身份/文件工具。软件包、服务、账号和 Git 模块调用对应系统工具；DNF/Yum 的操作系统 Python 依赖不属于 Rustsible 自身运行依赖。

## 模块

表由 `cargo run --locked -p rustsible-protocol --example module_matrix` 生成，参数关系和具体支持状态仍按统一校验器执行。内置短名、`ansible.builtin.*`、`ansible.legacy.*` 共用入口；`systemd_service` 是 `systemd` 别名。`set_fact` 的映射键是用户定义变量，表中无固定键。check 的“预测”有下述限制。

| 模块 | 参数 | check | diff | async |
|---|---|---|---|---|
| command | cmd, argv, chdir, creates, removes, stdin, stdin_add_newline, strip_empty_ends, expand_argument_vars | 预测 | — | 支持 |
| shell | cmd, argv, chdir, creates, removes, stdin, stdin_add_newline, strip_empty_ends, expand_argument_vars | 预测 | — | 支持 |
| debug | msg, var | 只读 | — | — |
| meta | action | 只读 | — | — |
| set_fact |  | 只读 | — | — |
| include_vars | file, dir, name | 只读 | — | — |
| copy | src, content, dest, mode, owner, group, force | 预测 | 支持 | — |
| file | path, dest, state, src, mode, owner, group | 预测 | 支持 | 支持 |
| template | src, content, dest, mode, owner, group, vars | 预测 | 支持 | — |
| lineinfile | path, line, regexp, state, backup, create, insertafter, insertbefore, owner, group, mode | 预测 | 支持 | 支持 |
| user | name, state, uid, gid, group, groups, append, home, shell, comment, password, create_home, system, remove | 预测 | — | 支持 |
| service | name, state, enabled | 预测 | — | 支持 |
| systemd | name, state, enabled, daemon_reload | 预测 | — | 支持 |
| setup | gather_subset, filter, gather_timeout | 只读 | — | — |
| stat | path, checksum_algorithm | 只读 | — | — |
| get_url | url, dest, owner, group, mode, force, checksum | 预测 | 支持 | 支持 |
| package | name, state, update_cache | 预测 | — | 支持 |
| async_status | jid, mode | 预测 | — | — |
| assert | that, fail_msg, msg, success_msg, quiet | 只读 | — | — |
| fail | msg | 只读 | — | — |
| group | name, gid, state, system | 预测 | — | — |
| wait_for | host, port, path, state, search_regex, timeout, connect_timeout, delay, sleep | 只读 | — | — |
| uri | url, method, headers, body, body_format, status_code, timeout, return_content, validate_certs, use_proxy, ca_path, follow_redirects, url_username, url_password, user, password, force_basic_auth | 预测 | — | — |
| git | repo, dest, version, update | 预测 | 支持 | — |
| unarchive | src, dest, remote_src, owner, group, mode, creates | 预测 | 支持 | — |

- command/shell 缺少 creates/removes 时 check 跳过；不会执行命令。shell 解释用户明确提供的 shell 程序，其余系统工具使用参数数组。
- wait_for/uri 在 check 下跳过。async 在 check 下跳过且不创建后台任务。async 不是控制端上传的后台化方式；copy/template 和控制端 unarchive payload 不支持 async。
- diff 由支持模块结构化返回，文本每侧最多 64 KiB；大文件/二进制只显示摘要；no_log 及受保护 Vault 输入隐藏敏感内容。
- package 支持实际系统上的 apt、dnf、yum、apk、pacman、zypper；service 支持实际 systemd、OpenRC、SysV、Upstart。缺少工具时报错，不自动安装；check 不刷新缓存或改变服务状态。
- git 支持固定 revision/分支/标签、update、保留本地修改的拒绝行为；unarchive 支持 tar、tar.gz、ZIP，拒绝链接、设备、越界写入，预解码后再安装。
- setup 的 subset/filter 只支持注册表明确列出的平台、发行版、系统、架构、主机名、网络和时间 facts；不伪造控制端时间，不包含整个 Ansible facts 集合。
- get_url/uri 使用目标端 DNS、代理与证书信任；响应/内容匹配有明确边界。大文件下载、文件传输以流传递，不塞入协议 JSON。

## Playbook 与变量

| 类别 | 支持 |
|---|---|
| 运行状态 | 跨 play 的失败、不可达、register、facts；play/task 局部变量作用域；显式清除错误 |
| 变量来源 | inventory、group_vars/host_vars、vars_files、include_vars、set_fact、role defaults/vars、play/task vars、CLI -e；hostvars/groups/group_names/路径上下文 |
| 模板 | MiniJinja、严格布尔条件、原生单表达式类型、omit、自定义已声明过滤器/结果测试；外部输出及普通数据不再次编译 |
| 任务语义 | args、when、register、loop/with_items、loop_control、changed_when/failed_when、until/retries/delay、timeout、environment、become/become_user、connection、check_mode、no_log |
| 复用 | import_playbook/import_tasks/import_role、include_tasks/include_role、roles tasks/defaults/vars/handlers/meta 依赖；来源路径和环检测 |
| 失败处理 | block/rescue/always、ignore_errors、ignore_unreachable；handler notify/listen、去重、force_handlers、pre/post 阶段刷新 |
| meta | flush_handlers、clear_host_errors、reset_connection、end_host、end_play |
| 调度 | linear/free、serial 数量/百分比/列表、throttle、run_once、delegate_to/delegate_facts、any_errors_fatal/max_fail_percentage |
| 选择与预览 | tags/skip-tags、syntax-check、list-hosts、list-tasks、diff；语法/列表不连接主机 |
| 异步 | task async/poll、ad-hoc -B/-P、async_status/finished；按身份隔离，超时回收后代，显式或自动清理 |
| Vault | AES256 1.1/1.2、文件级加密、!vault、多 vault ID、密码文件/隐藏提示、encrypt/decrypt/view/rekey；默认拒绝密码执行脚本 |
| 扩展 | 见 NATIVE_EXTENSIONS.md；原生 module/inventory/lookup/filter，声明式 collection 内容，受控安装/配置；其他类别为限定的编译期 Rust 接口 |

free 不支持 run_once/any_errors_fatal/max_fail_percentage 组合，执行前报错。run_once 按 serial 批次执行。`fail_fast` 是 Rustsible 扩展。角色 argument_specs、Python 插件、未实现的模块、任务关键词和配置项明确报错，不静默吞掉。Windows/macOS、常驻 Agent、Galaxy Python 兼容层不在范围内。

## 配置与 CLI

配置文件优先级：显式 `--config`、`ANSIBLE_CONFIG`、项目、用户、系统。只接受已实现项：`[defaults] inventory/worker_dir/collections_path/roles_path/forks/verbosity/remote_user/private_key_file/timeout/host_key_checking`，`[privilege_escalation] become/become_user/become_method`（配置文件中的 method 仅 sudo）；其他配置项报错。CLI 普通开关覆盖对应配置；连接变量从最终变量上下文解析，extra vars 保持最高普通变量优先级。SSH/sudo 密码从安全提示或变量输入，不进入 argv/日志。

命令保留 playbook/ad-hoc/inventory-debug 组织；新增 vault、collection 和只读 JSON `build-info`。`--worker-dir` 指定离线 worker 产物目录，默认可执行文件旁 workers/；`rustsible_remote_tmp` 指定目标临时目录。`RELEASE.json` 记录源码提交、控制端身份/摘要、双架构 worker 清单；`SHA256SUMS` 覆盖最终包文件。

## 安全差异及证据

全局 --check 不能由 task check_mode:false 取消；允许临时引导上传并清理，禁止业务修改和异步启动。修改中断后结果无法确认则报告 uncertain，不自动重放。原子文件替换保留未指定权限/属主及保护元数据，失败保留原文件；无法安全处理时明确报错。

CI 的 ubuntu-24.04 与 ubuntu-24.04-arm 分别提供原生 x86_64/aarch64 运行证据，glibc/musl SSH 测试覆盖 deploy/root/app。systemd/OpenRC/SysV/Upstart 与包管理器另有真实运行环境，不用普通容器模拟代替服务验收。最终包组合场景验证下载校验→角色→Vault 模板→handler→三机滚动→健康检查→救援；检查实际目标字节、权限、身份及清理。跨架构控制场景单独注明 QEMU 控制端与原生目标，不能代替上述双架构原生执行证据。进度和通过的确切 CI 提交见 DEVELOPMENT_STATUS.md；未通过项不得标为完成。
