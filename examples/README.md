# Rustsible 示例 Playbook

这个目录包含一些示例 playbook，展示了如何使用 Rustsible 进行各种自动化任务。

## 目录结构

```
examples/
├── inventory/    # 主机清单文件
│   └── hosts     # 默认主机清单文件
├── playbooks/    # Playbook 定义文件
│   ├── node_exporter.yml    # 批量安装 Prometheus Node Exporter
│   ├── k8s_cluster.yml      # 使用 kubeadm 安装 Kubernetes 集群
│   ├── software_install.yml # 批量下载和安装软件
│   └── service_config.yml   # 批量配置和启动服务
├── templates/    # Jinja2 模板文件
│   ├── nginx.conf.j2          # Nginx 主配置模板
│   ├── nginx_vhost.conf.j2    # Nginx 虚拟主机模板
│   └── node_exporter.service.j2  # Node Exporter 服务模板
├── tasks/        # 可重用的任务文件
│   └── install_software.yml   # 软件安装任务
└── files/        # 静态文件
```

## 示例清单

- **node_exporter.yml**: 批量安装 Prometheus Node Exporter
- **k8s_cluster.yml**: 使用 kubeadm 安装 Kubernetes 集群
- **software_install.yml**: 批量下载和安装软件
- **service_config.yml**: 批量配置和启动服务

## 使用方法

所有示例都可以使用 Rustsible 的 playbook 命令运行：

```bash
# 使用示例 inventory 文件运行 playbook
rustsible playbook -i examples/inventory/hosts examples/playbooks/node_exporter.yml

# 使用自定义 inventory 文件
rustsible playbook -i your_inventory.ini examples/playbooks/k8s_cluster.yml
```

## 自定义变量

所有 playbook 都包含默认变量，您可以在执行时覆盖这些变量：

```bash
# 使用自定义变量
rustsible playbook -i examples/inventory/hosts examples/playbooks/service_config.yml -e "nginx_worker_processes=4 nginx_worker_connections=2048"
```

## 针对特定主机组运行

您可以使用 `-l` 或 `--limit` 选项限制 playbook 在特定主机上运行：

```bash
# 仅在 web1.example.com 上运行
rustsible playbook -i examples/inventory/hosts examples/playbooks/service_config.yml -l web1.example.com
```

## 运行临时命令

```bash
# 在 web 服务器上执行命令
rustsible ad-hoc -i examples/inventory/hosts web_servers -m command -a "uptime"

# 使用 shell 模块在数据库服务器上执行命令
rustsible ad-hoc -i examples/inventory/hosts db_servers -m shell -a "ps aux | grep mysql"

# 使用文件模块创建目录
rustsible ad-hoc -i examples/inventory/hosts all -m file -a "path=/tmp/rustsible-test state=directory"
```

## 注意事项

- 示例配置假设目标主机运行的是基于 Debian/Ubuntu 的操作系统
- 执行 playbook 前请确保您可以通过 SSH 密钥或密码访问目标主机
- 部分模块如 `selinux`、`systemd` 可能需要 Python 模块支持，请确保目标系统已安装
- 请根据您的实际环境修改主机清单和变量 