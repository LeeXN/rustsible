# Rustsible 示例

这个目录展示 Rustsible 当前注册的模块和 playbook 写法。项目并非 Ansible 的
完整替代品；运行示例前，请先使用 `--check`，并把 inventory 中的占位主机和
凭据替换为测试环境配置。

## 目录

```text
examples/
├── inventory/hosts
├── playbooks/
│   ├── test_all_features.yml
│   ├── test_all_modules.yml
│   ├── test_command.yml
│   ├── test_lineinfile.yml
│   ├── test_package.yml
│   ├── test_service.yml
│   ├── test_shell.yml
│   └── test_user.yml
└── templates/motd.j2
```

`test_all_features.yml` 和 `test_package.yml` 是迁移审查素材，刻意保留了
`group` 等尚未实现的 Ansible 功能，当前会被明确拒绝。其余文件
用于当前子集的示例，但也不构成完整兼容性保证。遇到不支持的模块、关键字或表达式时，
Rustsible 会返回错误，而不会静默忽略。

`reuse/main.yml` 展示 playbook 导入、角色模板与参数、动态任务文件、listen
handler 和 block/rescue/always。用本地 inventory 运行，文件仅写入
`/tmp/rustsible-reuse-demo`；可用 `-e demo_output=/其他测试目录` 改路径。
`--check` 下 command 不执行，救援分支不会为了示例而制造失败。

`rolling/playbook.yml` 与 `rolling/inventory` 展示三台逻辑主机按 `[1, 2]`
滚动执行、批次内 run_once、配置 handler、健康读取和委派到控制端的回执。
默认 inventory 使用 local，文件仅写入 `/tmp/rustsible-rolling-demo*`。
替换成 SSH inventory 后可用于隔离目标；三台独立 SSH 服务器的验收在
`tests/ssh/strategy.sh`。运行示例：

```bash
rustsible playbook examples/rolling/playbook.yml -i examples/rolling/inventory --check
rustsible playbook examples/rolling/playbook.yml -i examples/rolling/inventory
```

`async/playbook.yml` 与 `async/inventory` 展示后台启动、下一任务解锁执行、
until 查询和显式清理。默认文件位于 `/tmp/rustsible-async-demo`，可用
`-e demo_root=/其他测试目录` 改路径；全局 `--check` 不启动任务。

```bash
rustsible playbook examples/async/playbook.yml -i examples/async/inventory --check
rustsible playbook examples/async/playbook.yml -i examples/async/inventory
```

`assertions.yml` 展示原始布尔表达式、断言消息和 fail/rescue。使用本地
inventory 即可验证，不创建业务文件。`group` 会修改目标组数据库，完整的
创建、GID 更新、幂等、删除和 check 场景在隔离 SSH fixture 中运行。

## 运行

```bash
# 先预览，不写入目标机器
rustsible playbook examples/playbooks/test_command.yml \
  -i examples/inventory/hosts --limit localhost --check

# 确认目标和变更后再执行
rustsible playbook examples/playbooks/test_command.yml \
  -i examples/inventory/hosts --limit localhost

# 临时命令
rustsible ad-hoc localhost -i examples/inventory/hosts \
  -m command -a "uptime"
```

`command` 按参数列表执行，不解释管道或重定向；默认展开环境变量，但不会启动 shell。需要这些 shell
语法时请显式使用 `shell`。远程 SSH 默认严格校验 host key。

`network.yml` 展示 TCP 就绪等待和 HTTP 健康查询重试。实际执行前将
health_host/health_port/health_url 配成隔离服务；默认地址为
`http://127.0.0.1:8080/health`，示例不自动启动服务。

## 安全提示
- 示例 inventory 只应放测试值，不要提交真实密码或私钥。
- `package`、`service` 和 `user` 会调用目标系统工具，通常需要 `become`。
- `--check` 是最佳努力的预测，不替代隔离环境中的实际验证。
- 修改用户、服务、软件包或 `/etc` 下文件前，请准备可恢复方案。
