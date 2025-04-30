# Rustsible

**Ansible-Compatible, High-Performance IT Automation in Rust**

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/LeeXN/rustsible)

> ⚠️ **Development Status**: Rustsible is currently under active development. While many core features are functional, some features may be incomplete or subject to change. Use in production environments at your own discretion.

---

## Overview

Rustsible is a modern, Ansible-compatible automation tool written in Rust. It aims to be a drop-in replacement for Ansible, providing improved performance, safety, and maintainability. Rustsible leverages Rust's async and concurrency features, and is designed with modularity, type safety, and extensibility in mind.

---

## Features

- **Ansible-Compatible**: Supports standard Ansible playbooks and inventory formats
- **High Performance**: Built with Rust, optimized for speed and memory safety
- **Async & Concurrency**: Uses `tokio` for async task execution and efficient resource management
- **Modular Architecture**: Each module (e.g., file, copy, template) is cleanly separated and reusable
- **Local & Remote Execution**: Seamless execution on localhost and remote hosts via SSH
- **Type-Safe Parameter Extraction**: Generic, type-safe parameter extraction utilities for robust module development
- **Minimal Dependencies**: Distributed as a single binary, no Python or external runtime required
- **Cross-Platform**: Works on Linux, macOS, and Windows

---

## Quick Start

### Build from Source

```bash
# Clone the repository
git clone https://github.com/LeeXN/rustsible.git
cd rustsible

# Build release binary
cargo build --release
```

The binary will be at `target/release/rustsible`.

### Usage Examples

#### Run a Playbook

```bash
rustsible playbook examples/playbooks/test_all_modules.yml -i inventory
```

#### Run Ad-hoc Commands

```bash
# Basic command execution
rustsible ad-hoc localhost -m command -a "uname -a"

# File operations
rustsible ad-hoc all -m file -a "path=/tmp/test state=touch mode=0644"

# User management
rustsible ad-hoc all -m user -a "name=testuser state=present shell=/bin/bash"

# Line file management
rustsible ad-hoc all -m lineinfile -a "path=/etc/hosts line='127.0.0.1 test.local' backup=true"
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

[local]
localhost ansible_connection=local

[all:vars]
ansible_user=admin
```

---

## Supported Modules

Rustsible supports a comprehensive set of modules compatible with Ansible. All modules support both local and remote execution.

### 1. command - Execute Commands
Execute simple system commands.

```yaml
- name: Check system information
  command: uname -a
```

### 2. shell - Execute Shell Commands
Execute complex shell commands with pipes and redirections.

```yaml
- name: Use shell with pipes
  shell: ps aux | grep nginx | head -5
```

### 3. debug - Debug Output
Output debug information and variable content.

```yaml
- name: Display variable
  debug:
    var: ansible_hostname

- name: Display message
  debug:
    msg: "Hello, {{ ansible_hostname }}!"
```

### 4. copy - Copy Files
Copy files to target hosts.

```yaml
- name: Copy configuration file
  copy:
    src: /local/path/config.yml
    dest: /remote/path/config.yml
    mode: "0644"
    owner: root
    group: root

- name: Create file with content
  copy:
    content: |
      server {
        listen 80;
        server_name example.com;
      }
    dest: /etc/nginx/sites-available/example
    mode: "0644"
```

### 5. file - File Management
Manage file and directory states and permissions.

```yaml
- name: Create directory
  file:
    path: /var/app/logs
    state: directory
    mode: "0755"
    owner: app
    group: app

- name: Remove file
  file:
    path: /tmp/old_file.txt
    state: absent

- name: Create empty file
  file:
    path: /var/log/app.log
    state: touch
    mode: "0644"
```

### 6. template - Template Rendering
Render files using the Tera template engine.

```yaml
- name: Generate configuration file
  template:
    src: templates/nginx.conf.j2
    dest: /etc/nginx/nginx.conf
    mode: "0644"
    vars:
      server_name: "{{ ansible_hostname }}"
      worker_processes: 4
```

### 7. lineinfile - Line Management ⭐ New Feature
Manage single lines in files, similar to Ansible's lineinfile module.

#### Basic Usage
```yaml
- name: Add configuration line
  lineinfile:
    path: /etc/hosts
    line: "192.168.1.100 myserver.local"
    backup: true

- name: Update line using regex
  lineinfile:
    path: /etc/ssh/sshd_config
    regexp: "^#?PermitRootLogin"
    line: "PermitRootLogin no"
    backup: true

- name: Insert line at specific position
  lineinfile:
    path: /etc/fstab
    line: "/dev/sdb1 /data ext4 defaults 0 2"
    insertafter: "^/dev/sda"

- name: Remove matching lines
  lineinfile:
    path: /etc/hosts
    regexp: ".*old-server.*"
    state: absent
```

#### Parameters
- `path`: Target file path (required)
- `line`: Line content to add or modify
- `regexp`: Regular expression to match lines for modification
- `state`: State, `present` (default) or `absent`
- `backup`: Create backup file, default `false`
- `create`: Create file if it doesn't exist, default `false`
- `insertafter`: Insert after matching line, supports `EOF`
- `insertbefore`: Insert before matching line, supports `BOF`
- `mode`: File permissions
- `owner`: File owner
- `group`: File group

### 8. user - User Management ⭐ New Feature
Manage system user accounts, similar to Ansible's user module.

#### Basic Usage
```yaml
- name: Create user
  user:
    name: myuser
    comment: "My Application User"
    shell: /bin/bash
    home: /home/myuser
    create_home: true
    state: present

- name: Create system user
  user:
    name: appuser
    system: true
    shell: /bin/false
    create_home: false
    comment: "Application service user"

- name: Add user to groups
  user:
    name: myuser
    groups: ["wheel", "docker"]
    append: true

- name: Modify user shell
  user:
    name: myuser
    shell: /bin/zsh

- name: Remove user
  user:
    name: olduser
    state: absent
    remove: true  # Also remove home directory
```

#### Parameters
- `name`: Username (required)
- `state`: State, `present` (default) or `absent`
- `uid`: User ID
- `gid`: Primary group ID
- `groups`: Additional groups list
- `append`: Append to existing groups, default `false`
- `home`: Home directory path
- `shell`: Login shell
- `comment`: User comment (GECOS)
- `password`: Encrypted password hash
- `create_home`: Create home directory, default `true`
- `system`: System user, default `false`
- `remove`: Remove home directory when deleting user, default `false`

### 9. service - Service Management
Manage system service states.

```yaml
- name: Start nginx service
  service:
    name: nginx
    state: started
    enabled: true

- name: Restart service
  service:
    name: mysql
    state: restarted
```

### 10. package - Package Management
Manage system packages.

```yaml
- name: Install package
  package:
    name: nginx
    state: present

- name: Install multiple packages
  package:
    name: ["git", "curl", "vim"]
    state: present
```

---

## Local Execution Support

All modules support local execution (localhost). When the target host is `localhost` or `127.0.0.1`, modules execute directly without SSH connections.

```yaml
- name: Local execution example
  hosts: localhost
  tasks:
    - name: Create local file
      file:
        path: /tmp/local_test.txt
        state: touch
    
    - name: Manage local user
      user:
        name: localuser
        state: present
```

---

## Ad-hoc Command Examples

All modules support ad-hoc command execution with automatic type conversion for parameters:

```bash
# Basic commands
rustsible ad-hoc all -m command -a "uname -a"
rustsible ad-hoc all -m shell -a "ps aux | grep nginx"

# File operations
rustsible ad-hoc all -m file -a "path=/tmp/test state=touch mode=0644"
rustsible ad-hoc all -m file -a "path=/var/app state=directory mode=0755"

# Copy files
rustsible ad-hoc all -m copy -a "src=/local/file dest=/remote/file mode=0644"
rustsible ad-hoc all -m copy -a "content='Hello World' dest=/tmp/hello.txt"

# Line file management
rustsible ad-hoc all -m lineinfile -a "path=/etc/hosts line='127.0.0.1 test.local' backup=true"
rustsible ad-hoc all -m lineinfile -a "path=/tmp/config.txt regexp='^setting=' line='setting=new_value'"

# User management
rustsible ad-hoc all -m user -a "name=testuser state=present shell=/bin/bash"
rustsible ad-hoc all -m user -a "name=testuser groups=wheel,docker append=true"
rustsible ad-hoc all -m user -a "name=testuser state=absent remove=true"

# Service management
rustsible ad-hoc all -m service -a "name=nginx state=started enabled=true"

# Package management
rustsible ad-hoc all -m package -a "name=curl state=present"

# Debug output
rustsible ad-hoc all -m debug -a "msg='Hello from Rustsible'"
```

📖 **For comprehensive ad-hoc examples**, see [examples/ad-hoc-examples.md](examples/ad-hoc-examples.md)

---

## Example Playbooks

Check the `examples/playbooks/` directory for comprehensive examples:

- `test_lineinfile.yml` - lineinfile module examples
- `test_user.yml` - user module examples  
- `test_all_modules.yml` - comprehensive module demonstration
- `test_all_features.yml` - advanced features showcase

### Sample Playbook

```yaml
---
- name: Complete System Setup
  hosts: all
  become: true
  vars:
    app_user: myapp
    app_dir: /opt/myapp
    config_file: "{{ app_dir }}/config.ini"
  
  tasks:
    # User management
    - name: Create application user
      user:
        name: "{{ app_user }}"
        system: true
        shell: /bin/false
        home: "{{ app_dir }}"
        create_home: true
    
    # Directory setup
    - name: Create application directories
      file:
        path: "{{ item }}"
        state: directory
        owner: "{{ app_user }}"
        group: "{{ app_user }}"
        mode: "0755"
      loop:
        - "{{ app_dir }}/logs"
        - "{{ app_dir }}/data"
        - "{{ app_dir }}/tmp"
    
    # Configuration management
    - name: Create base configuration
      copy:
        content: |
          [app]
          name=MyApplication
          version=1.0.0
          debug=false
          
          [database]
          host=localhost
          port=5432
        dest: "{{ config_file }}"
        owner: "{{ app_user }}"
        mode: "0640"
    
    - name: Configure application settings
      lineinfile:
        path: "{{ config_file }}"
        regexp: "^{{ item.key }}="
        line: "{{ item.key }}={{ item.value }}"
        backup: true
      loop:
        - { key: "debug", value: "true" }
        - { key: "log_level", value: "INFO" }
    
    # Template rendering
    - name: Generate startup script
      template:
        content: |
          #!/bin/bash
          # Startup script for {{ app_user }}
          # Generated on {{ ansible_date_time.date }}
          
          APP_USER="{{ app_user }}"
          APP_DIR="{{ app_dir }}"
          CONFIG_FILE="{{ config_file }}"
          
          echo "Starting application as $APP_USER"
          echo "Application directory: $APP_DIR"
          echo "Configuration file: $CONFIG_FILE"
        dest: "{{ app_dir }}/start.sh"
        mode: "0755"
        owner: "{{ app_user }}"
```

---

## Error Handling

All modules support comprehensive error handling:

```yaml
- name: Task that might fail
  user:
    name: testuser
    groups: ["nonexistent_group"]
  ignore_errors: true

- name: Conditional task
  lineinfile:
    path: /etc/config
    line: "setting=value"
  when: ansible_os_family == "RedHat"
```

---

## Best Practices

1. **Backup Important Files**: Use `backup: true` parameter for file modifications
2. **Permission Management**: Always explicitly set file permissions and ownership
3. **Idempotency**: Leverage module idempotency - repeated execution produces consistent results
4. **Error Handling**: Use `ignore_errors` or conditional statements for potentially failing tasks
5. **Variable Usage**: Make playbooks flexible with template variables and facts
6. **Testing**: Test playbooks in development environments before production deployment

---

## Architecture Highlights

- **Async Runtime**: All remote operations are async, using `tokio` for concurrency and efficiency
- **Channel-Based Communication**: Uses `tokio::sync` channels for task coordination
- **Type-Safe Parameter Extraction**: Generic, type-safe parameter utilities in `src/modules/param.rs`:

```rust
let path: String = get_param(args, "path")?;
let mode: Option<String> = get_optional_param(args, "mode");
let backup: bool = get_optional_param(args, "backup").unwrap_or(false);
```

- **Error Handling**: All modules use `anyhow::Result` and propagate errors with context
- **Extensible Modules**: Add new modules by implementing the required trait and registering in `src/modules/mod.rs`
- **Template Engine**: Uses Tera for powerful template rendering with Ansible-compatible syntax



---

## License

This project is licensed under the [MIT License](LICENSE).

---

## Acknowledgements

- Inspired by [Ansible](https://github.com/ansible/ansible)
- Built with 💜 in Rust
- Thanks to [Ansible](https://github.com/ansible/ansible) and the Rust community
- Cursor AI for the code completion and documentation