# Rustsible Ad-hoc Command Examples

This document provides comprehensive examples of using Rustsible's ad-hoc commands for quick automation tasks.

## Basic Usage

The general syntax for ad-hoc commands is:
```bash
rustsible ad-hoc <host-pattern> -m <module-name> -a "<module-arguments>"
```

## Command and Shell Modules

### Basic Commands
```bash
# System information
rustsible ad-hoc all -m command -a "uname -a"
rustsible ad-hoc localhost -m command -a "whoami"
rustsible ad-hoc all -m command -a "date"

# Check disk space
rustsible ad-hoc all -m command -a "df -h"

# Check memory usage
rustsible ad-hoc all -m command -a "free -h"

# Check running processes
rustsible ad-hoc all -m command -a "ps aux"
```

### Shell Commands with Pipes
```bash
# Find files with shell
rustsible ad-hoc all -m shell -a "find /var/log -name '*.log' | head -10"

# Count processes
rustsible ad-hoc all -m shell -a "ps aux | wc -l"

# Check specific service
rustsible ad-hoc all -m shell -a "ps aux | grep nginx | grep -v grep"
```

## File Management

### Creating Files and Directories
```bash
# Create a directory
rustsible ad-hoc all -m file -a "path=/tmp/test_dir state=directory mode=0755"

# Create an empty file
rustsible ad-hoc all -m file -a "path=/tmp/test_file.txt state=touch mode=0644"

# Remove a file
rustsible ad-hoc all -m file -a "path=/tmp/test_file.txt state=absent"

# Create directory with specific permissions
rustsible ad-hoc all -m file -a "path=/opt/myapp state=directory mode=0750 owner=root group=root"
```

### File Copying
```bash
# Copy a file
rustsible ad-hoc all -m copy -a "src=/local/path/file.txt dest=/remote/path/file.txt mode=0644"

# Create file with content
rustsible ad-hoc all -m copy -a "content='Hello World' dest=/tmp/hello.txt mode=0644"

# Create configuration file
rustsible ad-hoc all -m copy -a "content='server_name=localhost\nport=8080' dest=/tmp/config.txt mode=0600"
```

## Line File Management

### Adding Lines to Files
```bash
# Add a simple line
rustsible ad-hoc all -m lineinfile -a "path=/etc/hosts line='127.0.0.1 myapp.local' backup=true"

# Create file if it doesn't exist
rustsible ad-hoc all -m lineinfile -a "path=/tmp/new_config.txt line='setting=value' create=true"

# Insert line after a pattern
rustsible ad-hoc all -m lineinfile -a "path=/tmp/config.txt line='new_setting=true' insertafter='^port='"
```

### Updating Lines with Regex
```bash
# Update existing line
rustsible ad-hoc all -m lineinfile -a "path=/tmp/config.txt regexp='^server_name=' line='server_name=newhost'"

# Remove lines matching pattern
rustsible ad-hoc all -m lineinfile -a "path=/tmp/config.txt regexp='^old_setting=' state=absent"
```

## User Management

### Creating and Managing Users
```bash
# Create a user
rustsible ad-hoc all -m user -a "name=testuser state=present shell=/bin/bash"

# Create system user
rustsible ad-hoc all -m user -a "name=appuser system=true shell=/bin/false create_home=false"

# Add user to groups
rustsible ad-hoc all -m user -a "name=testuser groups=wheel,docker append=true"

# Change user shell
rustsible ad-hoc all -m user -a "name=testuser shell=/bin/zsh"

# Remove user
rustsible ad-hoc all -m user -a "name=testuser state=absent remove=true"
```

### User Information
```bash
# Check if user exists (using command module)
rustsible ad-hoc all -m command -a "id testuser"
```

## Package Management

### Installing Packages
```bash
# Install single package
rustsible ad-hoc all -m package -a "name=curl state=present"

# Install multiple packages (note: limited by argument parsing)
rustsible ad-hoc all -m package -a "name=git state=present"
rustsible ad-hoc all -m package -a "name=vim state=present"

# Remove package
rustsible ad-hoc all -m package -a "name=old_package state=absent"
```

## Service Management

### Managing Services
```bash
# Start a service
rustsible ad-hoc all -m service -a "name=nginx state=started"

# Stop a service
rustsible ad-hoc all -m service -a "name=nginx state=stopped"

# Restart a service
rustsible ad-hoc all -m service -a "name=nginx state=restarted"

# Enable service to start on boot
rustsible ad-hoc all -m service -a "name=nginx enabled=true"

# Start and enable service
rustsible ad-hoc all -m service -a "name=nginx state=started enabled=true"
```

## Debug and Information

### Debug Output
```bash
# Simple debug message
rustsible ad-hoc all -m debug -a "msg=Hello_World"

# Variable debugging (if variable exists)
rustsible ad-hoc all -m debug -a "var=ansible_hostname"
```

## Advanced Examples

### Comprehensive System Check
```bash
# Check system basics
rustsible ad-hoc all -m command -a "hostname"
rustsible ad-hoc all -m command -a "uptime"
rustsible ad-hoc all -m command -a "df -h /"
rustsible ad-hoc all -m command -a "free -m"
```

### Security Hardening Tasks
```bash
# Create security directory
rustsible ad-hoc all -m file -a "path=/etc/security/custom state=directory mode=0700"

# Add security configuration
rustsible ad-hoc all -m lineinfile -a "path=/etc/security/limits.conf line='* hard nofile 65536' backup=true"

# Create audit user
rustsible ad-hoc all -m user -a "name=audit system=true shell=/bin/false create_home=false"
```

### Application Deployment Tasks
```bash
# Create application structure
rustsible ad-hoc all -m file -a "path=/opt/myapp state=directory mode=0755"
rustsible ad-hoc all -m file -a "path=/opt/myapp/logs state=directory mode=0755"
rustsible ad-hoc all -m file -a "path=/opt/myapp/config state=directory mode=0755"

# Create application user
rustsible ad-hoc all -m user -a "name=myapp system=true home=/opt/myapp shell=/bin/bash"

# Create basic configuration
rustsible ad-hoc all -m copy -a "content='app_name=myapp\nversion=1.0.0\nport=8080' dest=/opt/myapp/config/app.conf mode=0644"

# Set ownership
rustsible ad-hoc all -m file -a "path=/opt/myapp owner=myapp group=myapp recurse=true"
```

### Log Management
```bash
# Create log directory
rustsible ad-hoc all -m file -a "path=/var/log/myapp state=directory mode=0755"

# Create log file
rustsible ad-hoc all -m copy -a "content='# Application Log Started\n' dest=/var/log/myapp/app.log mode=0644"

# Add log rotation configuration
rustsible ad-hoc all -m lineinfile -a "path=/etc/logrotate.d/myapp line='/var/log/myapp/*.log { daily rotate 7 compress }' create=true"
```

### Monitoring Setup
```bash
# Install monitoring tools
rustsible ad-hoc all -m package -a "name=htop state=present"

# Create monitoring script
rustsible ad-hoc all -m copy -a "content='#!/bin/bash\necho \"System: $(hostname)\"\necho \"Load: $(uptime)\"\necho \"Disk: $(df -h / | tail -1)\"' dest=/usr/local/bin/system-check mode=0755"

# Test monitoring script
rustsible ad-hoc all -m command -a "/usr/local/bin/system-check"
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
rustsible ad-hoc all -m command -a "ls -la /path/to/file"

# After creating a user
rustsible ad-hoc all -m command -a "id username"

# After modifying configuration
rustsible ad-hoc all -m command -a "cat /path/to/config"
```

## Common Use Cases

### Quick Server Setup
```bash
# Update package cache and install essentials
rustsible ad-hoc all -m package -a "name=curl state=present"
rustsible ad-hoc all -m package -a "name=wget state=present"
rustsible ad-hoc all -m package -a "name=vim state=present"
rustsible ad-hoc all -m package -a "name=htop state=present"

# Create admin user
rustsible ad-hoc all -m user -a "name=admin shell=/bin/bash create_home=true"
rustsible ad-hoc all -m user -a "name=admin groups=wheel append=true"

# Set up basic directory structure
rustsible ad-hoc all -m file -a "path=/opt/apps state=directory mode=0755"
rustsible ad-hoc all -m file -a "path=/var/log/apps state=directory mode=0755"
```

### Security Checks
```bash
# Check for suspicious files
rustsible ad-hoc all -m command -a "find /tmp -type f -perm /o+w"

# Check running services
rustsible ad-hoc all -m command -a "systemctl list-units --type=service --state=running"

# Check user accounts
rustsible ad-hoc all -m command -a "cat /etc/passwd | grep -v nologin | grep -v false"
```

### Maintenance Tasks
```bash
# Check disk usage
rustsible ad-hoc all -m command -a "du -sh /var/log/*"

# Clean temporary files
rustsible ad-hoc all -m command -a "find /tmp -type f -mtime +7 -delete"

# Check system logs
rustsible ad-hoc all -m command -a "tail -20 /var/log/messages"
```

This collection of examples should help you get started with Rustsible's ad-hoc commands for various automation tasks. 