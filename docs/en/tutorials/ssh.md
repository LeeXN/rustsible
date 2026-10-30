[English / 简体中文](../../zh-CN/tutorials/ssh.md) · [Index / 目录](../index.md)

# Deploy through SSH

<a id="prepare"></a>

## Prepare an isolated target

Use a disposable Linux host, an existing login account and a host key verified through a trusted channel. Replace every example address/path. The address below is documentation-only. Create site.yml using the complete example below.

<a id="inventory"></a>

## Inventory file

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

## Create site.yml

Save the following beside inventory.yml. Choose a scratch demo_root owned by the remote execution identity; the example creates it but must not be pointed at an unrelated existing directory. No package, service or system-account mutation is involved.

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

## Verify identity, then deploy

```bash
rustsible ad-hoc web -i inventory.yml -m command -a 'id -un'
rustsible playbook site.yml -i inventory.yml --list-hosts
rustsible playbook site.yml -i inventory.yml -e demo_root=/tmp/reviewed-app-demo --check --diff
rustsible playbook site.yml -i inventory.yml -e demo_root=/tmp/reviewed-app-demo --private-key /secure/deploy-key
```

<a id="privilege"></a>

## Privilege and temporary workers

Use ansible_become and ansible_become_user for task execution identity; the login identity remains separate. --ask-pass and --ask-become-pass require a controlling terminal and hide input. Do not disable host-key verification to fix a mismatch. rustsible_remote_tmp must name an existing executable, safe parent directory. Each identity gets its own private worker lease. See [connections](../guides/connections.md).
