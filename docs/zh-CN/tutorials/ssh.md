[English / 简体中文](../../en/tutorials/ssh.md) · [Index / 目录](../index.md)

# 通过 SSH 部署

<a id="prepare"></a>

## 准备隔离目标

准备一次性 Linux 主机、已有登录账号和通过可信渠道核验的主机密钥。替换所有示例地址与路径；下列地址仅用于文档。site.yml 是你审查后的 playbook，不会自动生成。

<a id="inventory"></a>

## 主机清单文件

```yaml
all:
  children:
    web:
      hosts:
        node1:
          ansible_host: 192.0.2.10
      vars:
        ansible_connection: ssh
        ansible_user: deploy
        ansible_ssh_private_key_file: /secure/deploy-key
        rustsible_known_hosts_file: /secure/known_hosts
```

<a id="run"></a>

## 先验证身份再部署

```bash
rustsible ad-hoc web -i inventory.yml -m command -a 'id -un'
rustsible playbook site.yml -i inventory.yml --list-hosts
rustsible playbook site.yml -i inventory.yml --check --diff
rustsible playbook site.yml -i inventory.yml --private-key /secure/deploy-key
```

<a id="privilege"></a>

## 提权与临时执行程序

ansible_become、ansible_become_user 控制任务执行身份，与登录身份分开。--ask-pass、--ask-become-pass 需要控制终端并隐藏输入。主机密钥不匹配时应核对来源，不应靠关闭校验解决。rustsible_remote_tmp 须指向已有、允许执行且安全的父目录；各执行身份持有独立私有 worker。详见[连接](../guides/connections.md)。
