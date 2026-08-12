# Rustsible Ad-hoc Command Examples

This document provides comprehensive examples of using Rustsible's ad-hoc commands for quick automation tasks.

## Basic Usage

The general syntax for ad-hoc commands is:
```bash
rustsible ad-hoc <host-pattern> -m <module-name> -a "<module-arguments>" -i examples/inventory/hosts
```

Commands that change users, packages, services, or protected paths such as
`/etc`, `/opt`, `/usr`, and `/var` require suitable privileges. Run those
examples as root, add `--become`, or set `ansible_become=true` for the target
hosts in inventory. Without one of those prerequisites, permission failures are
expected. Start with `--check` and a disposable target before applying changes.

## Command and Shell Modules

### Basic Commands
```bash
# System information
rustsible ad-hoc all -m command -a "uname -a" -i examples/inventory/hosts
rustsible ad-hoc localhost -m command -a "whoami" -i examples/inventory/hosts
rustsible ad-hoc all -m command -a "date" -i examples/inventory/hosts

# Check disk space
rustsible ad-hoc all -m command -a "df -h" -i examples/inventory/hosts

# Check memory usage
rustsible ad-hoc all -m command -a "free -h" -i examples/inventory/hosts

# Check running processes
rustsible ad-hoc all -m command -a "ps aux" -i examples/inventory/hosts
```

### Shell Commands with Pipes
```bash
# Find files with shell
rustsible ad-hoc all -m shell -a "find /var/log -name '*.log' | head -10" -i examples/inventory/hosts

# Count processes
rustsible ad-hoc all -m shell -a "ps aux | wc -l" -i examples/inventory/hosts

# Check specific service
rustsible ad-hoc all -m shell -a "ps aux | grep nginx | grep -v grep" -i examples/inventory/hosts
```

## File Management

### Creating Files and Directories
```bash
# Create a directory
rustsible ad-hoc all -m file -a "path=/tmp/test_dir state=directory mode=0755" -i examples/inventory/hosts

# Create an empty file
rustsible ad-hoc all -m file -a "path=/tmp/test_file.txt state=touch mode=0644" -i examples/inventory/hosts

# Remove a file
rustsible ad-hoc all -m file -a "path=/tmp/test_file.txt state=absent" -i examples/inventory/hosts

# Create directory with specific permissions
rustsible ad-hoc all -m file -a "path=/opt/myapp state=directory mode=0750 owner=root group=root" -i examples/inventory/hosts
```

### File Copying
```bash
# Copy a file
rustsible ad-hoc all -m copy -a "src=/local/path/file.txt dest=/remote/path/file.txt mode=0644" -i examples/inventory/hosts

# Create file with content
rustsible ad-hoc all -m copy -a "content='Hello World' dest=/tmp/hello.txt mode=0644" -i examples/inventory/hosts

# For multiline content, put the content in a local file and use src=...
rustsible ad-hoc all -m copy -a "src=examples/files/app.conf dest=/tmp/config.txt mode=0600" -i examples/inventory/hosts
```

## Line File Management

### Adding Lines to Files
```bash
# Add a simple line
rustsible ad-hoc all -m lineinfile -a "path=/etc/hosts line='127.0.0.1 myapp.local' backup=true" -i examples/inventory/hosts

# Create file if it doesn't exist
rustsible ad-hoc all -m lineinfile -a "path=/tmp/new_config.txt line='setting=value' create=true" -i examples/inventory/hosts

# Insert line after a pattern
rustsible ad-hoc all -m lineinfile -a "path=/tmp/config.txt line='new_setting=true' insertafter='^port='" -i examples/inventory/hosts
```

### Updating Lines with Regex
```bash
# Update existing line
rustsible ad-hoc all -m lineinfile -a "path=/tmp/config.txt regexp='^server_name=' line='server_name=newhost'" -i examples/inventory/hosts

# Remove lines matching pattern
rustsible ad-hoc all -m lineinfile -a "path=/tmp/config.txt regexp='^old_setting=' state=absent" -i examples/inventory/hosts
```

## User Management

### Creating and Managing Users
```bash
# Create a user
rustsible ad-hoc all -m user -a "name=testuser state=present shell=/bin/bash" -i examples/inventory/hosts

# Create system user
rustsible ad-hoc all -m user -a "name=appuser system=true shell=/bin/false create_home=false" -i examples/inventory/hosts

# Add user to groups
rustsible ad-hoc all -m user -a "name=testuser groups=wheel,docker append=true" -i examples/inventory/hosts

# Change user shell
rustsible ad-hoc all -m user -a "name=testuser shell=/bin/zsh" -i examples/inventory/hosts

# Remove user
rustsible ad-hoc all -m user -a "name=testuser state=absent remove=true" -i examples/inventory/hosts
```

### User Information
```bash
# Check if user exists (using command module)
rustsible ad-hoc all -m command -a "id testuser" -i examples/inventory/hosts
```

## Package Management

### Installing Packages
```bash
# Install single package
rustsible ad-hoc all -m package -a "name=curl state=present" -i examples/inventory/hosts

# Install multiple packages (note: limited by argument parsing)
rustsible ad-hoc all -m package -a "name=git state=present" -i examples/inventory/hosts
rustsible ad-hoc all -m package -a "name=vim state=present" -i examples/inventory/hosts

# Remove package
rustsible ad-hoc all -m package -a "name=old_package state=absent" -i examples/inventory/hosts
```

## Service Management

### Managing Services
```bash
# Start a service
rustsible ad-hoc all -m service -a "name=nginx state=started" -i examples/inventory/hosts

# Stop a service
rustsible ad-hoc all -m service -a "name=nginx state=stopped" -i examples/inventory/hosts

# Restart a service
rustsible ad-hoc all -m service -a "name=nginx state=restarted" -i examples/inventory/hosts

# Enable service to start on boot
rustsible ad-hoc all -m service -a "name=nginx enabled=true" -i examples/inventory/hosts

# Start and enable service
rustsible ad-hoc all -m service -a "name=nginx state=started enabled=true" -i examples/inventory/hosts
```

## Debug and Information

### Debug Output
```bash
# Simple debug message
rustsible ad-hoc all -m debug -a "msg=Hello_World" -i examples/inventory/hosts

# Ad-hoc debug supports literal messages; playbook variables require playbook context
rustsible ad-hoc all -m debug -a "msg=inventory_checked" -i examples/inventory/hosts
```

## Advanced Examples

### Comprehensive System Check
```bash
# Check system basics
rustsible ad-hoc all -m command -a "hostname" -i examples/inventory/hosts
rustsible ad-hoc all -m command -a "uptime" -i examples/inventory/hosts
rustsible ad-hoc all -m command -a "df -h /" -i examples/inventory/hosts
rustsible ad-hoc all -m command -a "free -m" -i examples/inventory/hosts
```

### Security Hardening Tasks
```bash
# Create security directory
rustsible ad-hoc all -m file -a "path=/etc/security/custom state=directory mode=0700" -i examples/inventory/hosts

# Add security configuration
rustsible ad-hoc all -m lineinfile -a "path=/etc/security/limits.conf line='* hard nofile 65536' backup=true" -i examples/inventory/hosts

# Create audit user
rustsible ad-hoc all -m user -a "name=audit system=true shell=/bin/false create_home=false" -i examples/inventory/hosts
```

### Application Deployment Tasks
```bash
# Create application structure
rustsible ad-hoc all -m file -a "path=/opt/myapp state=directory mode=0755" -i examples/inventory/hosts
rustsible ad-hoc all -m file -a "path=/opt/myapp/logs state=directory mode=0755" -i examples/inventory/hosts
rustsible ad-hoc all -m file -a "path=/opt/myapp/config state=directory mode=0755" -i examples/inventory/hosts

# Create application user
rustsible ad-hoc all -m user -a "name=myapp system=true home=/opt/myapp shell=/bin/bash" -i examples/inventory/hosts

# Copy a local configuration file (recursive ownership is not supported)
rustsible ad-hoc all -m copy -a "src=examples/files/app.conf dest=/opt/myapp/config/app.conf mode=0644 owner=myapp group=myapp" -i examples/inventory/hosts
```

### Log Management
```bash
# Create log directory
rustsible ad-hoc all -m file -a "path=/var/log/myapp state=directory mode=0755" -i examples/inventory/hosts

# Create log file
rustsible ad-hoc all -m copy -a "content='Application_Log_Started' dest=/var/log/myapp/app.log mode=0644" -i examples/inventory/hosts

# Add log rotation configuration
rustsible ad-hoc all -m lineinfile -a "path=/etc/logrotate.d/myapp line='/var/log/myapp/*.log { daily rotate 7 compress }' create=true" -i examples/inventory/hosts
```

### Monitoring Setup
```bash
# Install monitoring tools
rustsible ad-hoc all -m package -a "name=htop state=present" -i examples/inventory/hosts

# Copy a local script; this avoids controller-side shell expansion
rustsible ad-hoc all -m copy -a "src=examples/files/system-check dest=/usr/local/bin/system-check mode=0755" -i examples/inventory/hosts

# Test monitoring script
rustsible ad-hoc all -m command -a "/usr/local/bin/system-check" -i examples/inventory/hosts
```

## Tips and Best Practices

### 1. Parameter Handling
- Avoid spaces in parameter values when possible
- Use underscores instead of spaces: `msg=Hello_World` instead of `msg=Hello World`
- For complex content, consider using playbooks instead

### 2. Error Handling
- Most ad-hoc commands will show errors immediately
- Use `ignore_errors=true` in playbooks for better error handling

### 3. File Permissions
- Always specify file permissions explicitly: `mode=0644`
- Use appropriate permissions for security: `mode=0600` for sensitive files

### 4. Backup Important Files
- Use `backup=true` with lineinfile for important configuration files
- Test changes on non-production systems first

### 5. Verification
- Always verify changes with follow-up commands:
```bash
# After creating a file
rustsible ad-hoc all -m command -a "ls -la /path/to/file" -i examples/inventory/hosts

# After creating a user
rustsible ad-hoc all -m command -a "id username" -i examples/inventory/hosts

# After modifying configuration
rustsible ad-hoc all -m command -a "cat /path/to/config" -i examples/inventory/hosts
```

## Common Use Cases

### Quick Server Setup
```bash
# Update package cache and install essentials
rustsible ad-hoc all -m package -a "name=curl state=present" -i examples/inventory/hosts
rustsible ad-hoc all -m package -a "name=wget state=present" -i examples/inventory/hosts
rustsible ad-hoc all -m package -a "name=vim state=present" -i examples/inventory/hosts
rustsible ad-hoc all -m package -a "name=htop state=present" -i examples/inventory/hosts

# Create admin user
rustsible ad-hoc all -m user -a "name=admin shell=/bin/bash create_home=true" -i examples/inventory/hosts
rustsible ad-hoc all -m user -a "name=admin groups=wheel append=true" -i examples/inventory/hosts

# Set up basic directory structure
rustsible ad-hoc all -m file -a "path=/opt/apps state=directory mode=0755" -i examples/inventory/hosts
rustsible ad-hoc all -m file -a "path=/var/log/apps state=directory mode=0755" -i examples/inventory/hosts
```

### Security Checks
```bash
# Check for suspicious files
rustsible ad-hoc all -m command -a "find /tmp -type f -perm /o+w" -i examples/inventory/hosts

# Check running services
rustsible ad-hoc all -m command -a "systemctl list-units --type=service --state=running" -i examples/inventory/hosts

# Check user accounts
rustsible ad-hoc all -m shell -a "cat /etc/passwd | grep -v nologin | grep -v false" -i examples/inventory/hosts
```

### Maintenance Tasks
```bash
# Check disk usage
rustsible ad-hoc all -m shell -a "du -sh /var/log/*" -i examples/inventory/hosts

# Clean temporary files
rustsible ad-hoc all -m command -a "find /tmp -type f -mtime +7 -delete" -i examples/inventory/hosts

# Check system logs
rustsible ad-hoc all -m command -a "tail -20 /var/log/messages" -i examples/inventory/hosts
```

This collection of examples should help you get started with Rustsible's ad-hoc commands for various automation tasks.
