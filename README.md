# Rustsible

[![CI](https://github.com/LeeXN/rustsible/actions/workflows/rust.yml/badge.svg)](https://github.com/LeeXN/rustsible/actions/workflows/rust.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

Rustsible is an experimental, Ansible-inspired configuration and command runner written in Rust.

> Rustsible is not a drop-in replacement for Ansible. It implements a useful subset of Ansible's inventory, playbook, templating, and module behavior, but compatibility is incomplete. Review a playbook with `--check` and test it in a disposable environment before using it on important systems.

## Current scope

Rustsible currently provides:

- INI and YAML inventory parsing, group variables, child groups, and host-pattern filtering;
- playbook tasks, variables, loops, conditions, registered results, handlers, and `become` support;
- ad-hoc module execution;
- local command execution and remote execution through SSH;
- playbook support for `command`, `shell`, `debug`, `copy`, `file`, `template`, `lineinfile`, `user`, `service`, `systemd`, `stat`, `get_url`, and `package`;
- short module names and the `ansible.builtin.*` / `ansible.legacy.*` name prefixes;
- playbook `--limit`, `--check`, and `--forks` options.

Tasks retain playbook order, while hosts within a task run concurrently with a bounded worker count (`--forks`, default 5). Most state-management modules and remote commands assume a POSIX shell, and the `package`, `service`, and `user` modules target Linux-style system tools. Linux is the primary supported and tested platform; Windows support is not claimed.

Module names and common parameters resemble Ansible, but their complete argument schemas, facts, plugin system, transports, strategy engine, Vault support, and all Ansible edge cases are not implemented. Idempotency and check-mode prediction can also differ from Ansible.

Playbook fact gathering follows the ansible-core 2.21 default: facts are gathered
unless a play sets `gather_facts: false`. Fact gathering, `stat`, and `get_url`
require Python 3 on managed hosts.
The compatibility surface currently includes Linux architecture facts,
`stat.path/checksum_algorithm`, `get_url.url/dest/owner/group/mode/force/checksum`
(inline `<algorithm>:<digest>` checksums),
`systemd.name/enabled/state/daemon_reload`, `copy.force`, and `default(omit)`.
The systemd module rejects non-systemd hosts explicitly.

## Build

Install a current stable Rust toolchain, then run:

```bash
git clone https://github.com/LeeXN/rustsible.git
cd rustsible
cargo build --release --locked
```

The binary is written to `target/release/rustsible`.

## Usage

Run a playbook:

```bash
rustsible playbook site.yml -i inventory.ini
```

Limit execution to a host or group expression:

```bash
rustsible playbook site.yml -i inventory.ini --limit webservers
```

Preview a playbook without applying state changes:

```bash
rustsible playbook site.yml -i inventory.ini --check
```

In check mode, commands with no safe prediction (`command` and `shell`) are skipped. State-management modules report their predicted result without applying it. Prediction is best effort and is not a substitute for testing against a disposable target.

Control the maximum number of hosts processed concurrently:

```bash
rustsible playbook site.yml -i inventory.ini --forks 10
```

Run an ad-hoc module:

```bash
rustsible ad-hoc webservers -i inventory.ini -m command -a "uname -a"
rustsible ad-hoc all -i inventory.ini -m file -a "path=/tmp/example state=touch mode=0644"
rustsible ad-hoc all -i inventory.ini -m package -a "name=curl state=present" --become --check
rustsible ad-hoc all -i inventory.ini -m ansible.builtin.setup
rustsible ad-hoc all -i inventory.ini -m stat -a '{"path":"/usr/local/bin/rs-cmdb-client","checksum_algorithm":"sha1"}'
```

Ad-hoc runs inherit `ansible_become` and `ansible_become_user` per host. Use
`--become`, `--become-user`, `--check`, and `--forks` to override execution
controls from the command line. Module arguments accept Ansible-style
`key=value` input or a JSON object. `command` and `shell` additionally accept a
JSON string or `{"cmd":"..."}`. Structured module data such as `ansible_facts`,
`stat`, and `checksum_src` is printed as JSON. As in Ansible, ad-hoc fact
gathering uses the `setup` module rather than the play-only `gather_facts`
keyword.

Inspect the parsed inventory without running tasks:

```bash
rustsible inventory-debug -i inventory.ini
```

Use `rustsible <subcommand> --help` for the complete CLI syntax.

## Inventory

A minimal INI inventory looks like this:

```ini
[webservers]
web1 ansible_host=192.0.2.10 ansible_user=deploy
web2 ansible_host=192.0.2.11 ansible_port=2222

[local]
localhost ansible_connection=local

[webservers:vars]
ansible_ssh_private_key_file=/home/deploy/.ssh/id_ed25519
```

YAML inventory is also accepted:

```yaml
all:
  children:
    webservers:
      hosts:
        web1:
          ansible_host: 192.0.2.10
          ansible_user: deploy
```

Inventory variables may contain credentials. Keep inventory files out of source control, restrict their file permissions, and prefer SSH keys over passwords.

## Playbook example

```yaml
---
- name: Configure web nodes
  hosts: webservers
  become: true
  vars:
    config_path: /etc/example.conf

  tasks:
    - name: Install the package
      package:
        name: nginx
        state: present

    - name: Render configuration
      template:
        src: templates/example.conf.j2
        dest: "{{ config_path }}"
        mode: "0644"
      notify: restart nginx

  handlers:
    - name: restart nginx
      service:
        name: nginx
        state: restarted
```

Examples under [`examples/`](examples/) include runnable subset examples plus explicitly documented migration fixtures that exercise unsupported Ansible syntax. They are examples rather than a compatibility guarantee.

## SSH security defaults

SSH host-key verification is enabled by default. Rustsible reads the system OpenSSH known-hosts file and `$HOME/.ssh/known_hosts`; an unknown or mismatched key causes the connection to fail.

For an isolated known-hosts file, set one of these inventory variables:

```ini
host1 rustsible_known_hosts_file=/path/to/known_hosts
# ansible_ssh_known_hosts_file is also recognized
```

Host-key verification can be disabled only through an explicit inventory setting:

```ini
host1 rustsible_host_key_checking=false
```

`ansible_host_key_checking` and `ansible_ssh_host_key_checking` are also recognized. Disabling verification makes SSH connections vulnerable to interception and should only be used in controlled, temporary environments.

The default SSH connection timeout is 10 seconds and the default remote-command timeout is 300 seconds. They can be configured with `ansible_ssh_timeout` / `ansible_timeout` and `rustsible_command_timeout` / `ansible_command_timeout`, respectively.

## Supported modules

The following built-in modules are registered for playbooks:

| Module | Purpose |
| --- | --- |
| `command` | Run a command |
| `shell` | Run a command through a shell |
| `debug` | Display a message or variable |
| `copy` | Copy a controller file or literal content |
| `file` | Manage files, directories, links, and permissions |
| `template` | Render a Tera/Jinja-like template |
| `lineinfile` | Add, replace, or remove matching lines |
| `user` | Manage local user accounts |
| `service` | Manage service state and enablement |
| `systemd` / `systemd_service` | Manage systemd units and daemon reloads |
| `setup` | Gather the supported Linux facts |
| `stat` | Return path existence and checksums |
| `get_url` | Download HTTP/HTTPS resources on the managed host |
| `package` | Manage packages through detected system tools |

Only the subset exercised by this repository's tests should be considered supported. Consult the module source and examples before relying on Ansible-specific parameters.

## Development and testing

The CI-equivalent local checks are:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --all-features --locked
cargo llvm-cov --all-targets --all-features --locked --fail-under-lines 80
cargo build --release --all-features --locked
```

To run a focused test, pass its name to Cargo, for example:

```bash
cargo test modules::file
```

For a staged hands-on acceptance run covering local execution, SSH, check mode,
idempotency, privilege escalation, failure handling, concurrency, and log
redaction, follow [`MANUAL_TEST_PLAN.md`](MANUAL_TEST_PLAN.md).

Dependency advisories are checked in CI against `Cargo.lock`. CI also enforces
an 80% line-coverage floor; the current all-target result is 81.12%. This is a
regression baseline, not a claim that every remote SSH path is exercised end to
end.

## License

Rustsible is available under the [MIT License](LICENSE).

## Acknowledgements

Rustsible is inspired by [Ansible](https://github.com/ansible/ansible) and uses names familiar to Ansible users. It is an independent implementation and is not affiliated with or endorsed by Red Hat or the Ansible project.
