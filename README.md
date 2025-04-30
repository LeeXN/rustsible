# Rustsible
Ansible-Compatible, High-Performance IT Automation in Rust

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

## Table of Contents
- [Rustsible](#rustsible)
  - [Table of Contents](#table-of-contents)
  - [Introduction](#introduction)
  - [Features](#features)
  - [Installation](#installation)
    - [From Source](#from-source)
    - [Pre-built Binaries (coming soon)](#pre-built-binaries-coming-soon)
  - [Usage](#usage)
    - [Running Playbooks](#running-playbooks)
    - [Ad-hoc Commands](#ad-hoc-commands)
  - [Inventory Format](#inventory-format)
  - [Playbook Format](#playbook-format)
  - [Supported Modules](#supported-modules)
  - [Comparison to Ansible](#comparison-to-ansible)
  - [Examples](#examples)
  - [Contributing](#contributing)
  - [License](#license)
  - [Acknowledgements](#acknowledgements)

## Introduction

Rustsible is a modern, Ansible-compatible automation tool written in Rust. It aims to provide a drop-in replacement for Ansible with improved performance, safety, and ease of use. You can run your existing playbooks and inventories without modification.

## Features

- Ansible-compatible playbooks and inventory support
- Blazing-fast execution powered by Rust
- Parallel task execution across multiple hosts
- Minimal external dependencies (standalone binary)
- Secure by default with Rust’s memory safety guarantees
- Variable substitution via Jinja2-style (`{{ var }}`) and shell-style (`$var`)
- Privilege escalation (`become`/`sudo`) support
- Batch execution using the `serial` parameter
- Cross-platform: Linux, macOS, and Windows

## Installation

### From Source

```bash
# Clone the repository
git clone git@github.com:LeeXN/rustsible.git
cd rustsible
# Build release binary with Cargo
cargo build --release
```

The compiled binary will be available at `target/release/rustsible`.

### Pre-built Binaries (coming soon)

We plan to provide pre-built releases for major platforms under the [Releases](https://github.com/yourusername/rustsible/releases) page.

## Usage

Rustsible follows command-line conventions similar to Ansible.

### Running Playbooks

```bash
rustsible playbook <playbook.yml> -i <inventory>
```

### Ad-hoc Commands

```bash
rustsible ad-hoc <pattern> -m command -a "uptime" -i <inventory>
```

## Inventory Format

Rustsible uses the same inventory file format as Ansible.

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

## Playbook Format

Rustsible supports standard Ansible YAML playbooks.

```yaml
---
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

## Comparison to Ansible

- **Compiled Binary**: Rustsible is distributed as a single compiled executable, whereas Ansible is a Python application.
- **Parallelism by Default**: Tasks run in parallel across hosts unless overridden.
- **Performance**: Rustsible leverages Rust’s performance characteristics for faster task execution.
- **Feature Parity**: Most common Ansible features are implemented; some advanced modules or plugins may not be available yet.

## Examples

Browse the `examples/` directory for sample playbooks and usage patterns. You can run these samples as:

```bash
rustsible playbook examples/sample-playbook.yml -i examples/inventory
```

## Contributing

Contributions are welcome! Please follow these guidelines:

1. Fork the repository and create a feature branch.
2. Write tests for new functionality or bug fixes.
3. Follow the project’s coding conventions and format with `rustfmt`.
4. Open a Pull Request describing your changes.

## License

This project is licensed under the [GPL-3.0 License](LICENSE).

## Acknowledgements

- Inspired by [Ansible](https://github.com/ansible/ansible)
- Built with 💜 in Rust