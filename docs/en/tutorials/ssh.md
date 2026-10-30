[English / 简体中文](../../zh-CN/tutorials/ssh.md) · [Index / 目录](../index.md)

# Deploy through SSH

<a id="prepare"></a>

## Prepare an isolated target

Use a disposable Linux host, an existing login account and a host key verified through a trusted channel. Replace every example address/path. The address below is documentation-only. site.yml is your reviewed playbook, not an automatically created file.

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

<a id="run"></a>

## Verify identity, then deploy

```bash
rustsible ad-hoc web -i inventory.yml -m command -a 'id -un'
rustsible playbook site.yml -i inventory.yml --list-hosts
rustsible playbook site.yml -i inventory.yml --check --diff
rustsible playbook site.yml -i inventory.yml --private-key /secure/deploy-key
```

<a id="privilege"></a>

## Privilege and temporary workers

Use ansible_become and ansible_become_user for task execution identity; the login identity remains separate. --ask-pass and --ask-become-pass require a controlling terminal and hide input. Do not disable host-key verification to fix a mismatch. rustsible_remote_tmp must name an existing executable, safe parent directory. Each identity gets its own private worker lease. See [connections](../guides/connections.md).
