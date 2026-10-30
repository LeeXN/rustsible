[English / 简体中文](../../en/reference/configuration.md) · [Index / 目录](../index.md)

# 配置与环境变量

<a id="selection"></a>

## 文件选择与优先级

显式 --config FILE 优先，其后为 ANSIBLE_CONFIG、项目 ansible.cfg、~/.ansible.cfg、/etc/ansible/ansible.cfg。只加载首个选中文件，不合并所有后续文件；已识别环境变量覆盖文件，对应显式 CLI 覆盖默认配置。连接变量仍高于关键词/默认连接值。

<a id="keys"></a>

## 支持的配置项

[defaults]：inventory（单文件）、worker_dir、roles_path、collections_path、forks（正数，默认5）、verbosity（0–4）、remote_user、private_key_file、timeout（正数 SSH 秒数）、host_key_checking（默认 true）。[privilege_escalation]：become、become_user、become_method（仅 sudo）。未知 section/key、非法值会附带来源报错。文件内相对路径依据配置文件位置，环境路径依据 cwd。

<a id="environment"></a>

## 环境变量对应

ANSIBLE_INVENTORY→inventory；ANSIBLE_FORKS→forks；ANSIBLE_ROLES_PATH→roles_path；ANSIBLE_COLLECTIONS_PATH→collections_path；ANSIBLE_VERBOSITY→verbosity；ANSIBLE_REMOTE_USER→remote_user；ANSIBLE_PRIVATE_KEY_FILE→private_key_file；ANSIBLE_TIMEOUT→timeout；ANSIBLE_HOST_KEY_CHECKING→host_key_checking；ANSIBLE_BECOME/ANSIBLE_BECOME_USER/ANSIBLE_BECOME_METHOD 对应提权项。ANSIBLE_CONFIG 用于选择文件，不是普通配置键。

<a id="example"></a>

## 配置示例

```ini
[defaults]
inventory = inventory.yml
forks = 5
roles_path = roles
collections_path = collections
worker_dir = workers
host_key_checking = true

[privilege_escalation]
become = false
become_user = root
become_method = sudo
```

<a id="connection"></a>

## 有效连接变量

实际连接使用 ansible_host/user/port/connection、ansible_ssh_private_key_file/password；提权使用 ansible_become/user/password/method。rustsible_known_hosts_file、rustsible_host_key_checking 控制信任策略；rustsible_remote_tmp 选择目标 worker 父目录，rustsible_command_timeout 是默认模块预算，ansible_async_dir 为私有作业目录。不要混淆 ansible_become_user 与 ansible_user；配置 collection 路径也会授权已声明原生代码。
