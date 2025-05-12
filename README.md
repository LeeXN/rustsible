# Rustsible

**Ansible-Compatible, High-Performance IT Automation in Rust**

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/LeeXN/rustsible)

---

## Overview

Rustsible is a modern, Ansible-compatible automation tool written in Rust. It aims to be a drop-in replacement for Ansible, providing improved performance, safety, and maintainability. Rustsible leverages Rust's async and concurrency features, and is designed with modularity, type safety, and extensibility in mind.

---

## Features

- **Ansible-Compatible**: Supports standard Ansible playbooks and inventory formats.
- **High Performance**: Built with Rust, optimized for speed and memory safety.
- **Async & Concurrency**: Uses `tokio` for async task execution and efficient resource management.
- **Modular Architecture**: Each module (e.g., file, copy, template) is cleanly separated and reusable.
- **Type-Safe Parameter Extraction**: Generic, type-safe parameter extraction utilities for robust module development.
- **Minimal Dependencies**: Distributed as a single binary, no Python or external runtime required.
- **Cross-Platform**: Works on Linux, macOS, and Windows.

---

## Quick Start

### Build from Source

```sh
# Clone the repository
git clone https://github.com/LeeXN/rustsible.git
cd rustsible
# Build release binary
cargo build --release
```

The binary will be at `target/release/rustsible`.

### Usage

#### Run a Playbook

```sh
rustsible playbook <playbook.yml> -i <inventory>
```

#### Run an Ad-hoc Command

```sh
rustsible ad-hoc <pattern> -m command -a "uptime" -i <inventory>
```

---

## Inventory Example

```ini
[webservers]
web1.example.com
web2.example.com

[dbservers]
db1.example.com
db2.example.com

[all:vars]
ansible_user=admin
```

## Playbook Example

```yaml
- name: Deploy web application
  hosts: webservers
  become: true
  tasks:
    - name: Create document root directory
      file:
        path: /var/www/html
        state: directory
        mode: '0755'
    - name: Copy index.html
      copy:
        src: files/index.html
        dest: /var/www/html/index.html
```

---

## Supported Modules

- `command`: Execute commands on remote hosts
- `shell`: Run shell commands
- `file`: Manage file properties and state
- `copy`: Transfer files to remote locations
- `template`: Render templates with variable substitution
- `service`: Control system services (systemd, sysvinit, upstart)
- `package`: Install or remove packages (apt, yum, dnf, zypper, pacman)
- `local`: Execute tasks on the local machine
- `debug`: Print debug messages during playbook execution

---

## Architecture Highlights

- **Async Runtime**: All remote operations are async, using `tokio` for concurrency and efficiency.
- **Channel-Based Communication**: Uses `tokio::sync` channels for task coordination.
- **Type-Safe Parameter Extraction**: See `src/modules/param.rs` for generic, type-safe parameter utilities:

```rust
let path: String = get_param(args, "path")?;
let mode: Option<String> = get_optional_param(args, "mode");
```

- **Error Handling**: All modules use `anyhow::Result` and propagate errors with context.
- **Extensible Modules**: Add new modules by implementing the required trait and registering in `src/modules/mod.rs`.

---

## Contributing

1. Fork the repository and create a feature branch.
2. Write tests for new features or bug fixes.
3. Follow Rust best practices and format code with `rustfmt`.
4. Open a Pull Request with a clear description of your changes.

---

## License

This project is licensed under the [MIT License](LICENSE).

---

## Acknowledgements

- Inspired by [Ansible](https://github.com/ansible/ansible)
- Built with 💜 in Rust
