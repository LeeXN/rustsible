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
`group`、`set_fact` 等尚未实现的 Ansible 功能，当前会被明确拒绝。其余文件
用于当前子集的示例，但也不构成完整兼容性保证。遇到不支持的模块、关键字或表达式时，
Rustsible 会返回错误，而不会静默忽略。

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

`command` 按参数列表执行，不解释管道、重定向或变量展开；需要这些 shell
语法时请显式使用 `shell`。远程 SSH 默认严格校验 host key。

## 安全提示

- 示例 inventory 只应放测试值，不要提交真实密码或私钥。
- `package`、`service` 和 `user` 会调用目标系统工具，通常需要 `become`。
- `--check` 是最佳努力的预测，不替代隔离环境中的实际验证。
- 修改用户、服务、软件包或 `/etc` 下文件前，请准备可恢复方案。
