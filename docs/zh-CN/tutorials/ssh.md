[English / 简体中文](../../en/tutorials/ssh.md) · [Index / 目录](../index.md)

# 通过 SSH 部署

<a id="prepare"></a>

## 准备隔离目标

准备一次性 Linux 主机、已有登录账号和通过可信渠道核验的主机密钥。替换所有示例地址与路径；下列地址仅用于文档。按下文完整示例创建 site.yml。

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

<a id="playbook"></a>

## 准备 site.yml

将下列文件保存在 inventory.yml 旁。选择属于目标执行身份的独立 demo_root；示例会创建它，不要指向无关的已有目录。不涉及软件包、服务或系统账号变更。

```yaml
- hosts: web
  gather_facts: false
  tasks:
    - file:
        path: '{{ demo_root }}'
        state: directory
        mode: '0700'
    - copy:
        content: "managed=rustsible\n"
        dest: '{{ demo_root }}/config'
        mode: '0600'
    - stat:
        path: '{{ demo_root }}/config'
        checksum_algorithm: sha256
      register: installed
      when: not ansible_check_mode
    - assert:
        that: installed.stat.mode == '0600'
      when: not ansible_check_mode
```

<a id="run"></a>

## 先验证身份再部署

```bash
rustsible ad-hoc web -i inventory.yml -m command -a 'id -un'
rustsible playbook site.yml -i inventory.yml --list-hosts
rustsible playbook site.yml -i inventory.yml -e demo_root=/tmp/reviewed-app-demo --check --diff
rustsible playbook site.yml -i inventory.yml -e demo_root=/tmp/reviewed-app-demo --private-key /secure/deploy-key
```

<a id="privilege"></a>

## 提权与临时执行程序

ansible_become、ansible_become_user 控制任务执行身份，与登录身份分开。--ask-pass、--ask-become-pass 需要控制终端并隐藏输入。主机密钥不匹配时应核对来源，不应靠关闭校验解决。rustsible_remote_tmp 须指向已有、允许执行且安全的父目录；各执行身份持有独立私有 worker。详见[连接](../guides/connections.md)。
