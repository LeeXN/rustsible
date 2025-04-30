use std::fs::File;
use std::io::{self, BufRead};
use std::path::Path;
use anyhow::{Result, Context};
use regex::Regex;
use log::{debug, info};

use super::{Inventory, Host, HostGroup};

pub fn parse_inventory(inventory_path: &str) -> Result<Inventory> {
    let path = Path::new(inventory_path);
    
    if !path.exists() {
        return Err(anyhow::anyhow!("Inventory file not found: {}", inventory_path));
    }
    
    let file = File::open(path).context("Failed to open inventory file")?;
    let reader = io::BufReader::new(file);
    
    let mut inventory = Inventory::new();
    let mut current_group: Option<String> = None;
    
    // Regular expressions for parsing
    let group_re = Regex::new(r"^\[([\w.]+)(?::vars|:children)?\]$").unwrap();
    let group_vars_re = Regex::new(r"^\[([\w.]+):vars\]$").unwrap();
    let group_children_re = Regex::new(r"^\[([\w.]+):children\]$").unwrap();
    let host_port_re = Regex::new(r"^(.+):(\d+)$").unwrap();
    let var_line_re = Regex::new(r"^(\w+)=(.+)$").unwrap();
    
    info!("Parsing inventory file: {}", inventory_path);
    
    for (line_num, line_result) in reader.lines().enumerate() {
        let line = line_result.context(format!("Failed to read line {} from inventory file", line_num + 1))?;
        let line = line.trim();
        
        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        
        debug!("Processing line: {}", line);
        
        // Check if this is a group:vars section
        if let Some(caps) = group_vars_re.captures(line) {
            let group_name = caps.get(1).unwrap().as_str().trim();
            debug!("Found group vars section for group: {}", group_name);
            
            // Ensure the group exists
            if !inventory.get_group(group_name).is_some() {
                debug!("Creating group for vars section: {}", group_name);
                let group = HostGroup::new(group_name);
                inventory.add_group(group);
            }
            
            current_group = Some(format!("{}:vars", group_name));
            continue;
        }
        // Check if this is a group:children section
        else if let Some(caps) = group_children_re.captures(line) {
            let group_name = caps.get(1).unwrap().as_str().trim();
            debug!("Found group children section for group: {}", group_name);
            
            // Ensure the group exists
            if !inventory.get_group(group_name).is_some() {
                debug!("Creating group for children section: {}", group_name);
                let group = HostGroup::new(group_name);
                inventory.add_group(group);
            }
            
            current_group = Some(format!("{}:children", group_name));
            continue;
        }
        // Check if this is a regular group
        else if let Some(caps) = group_re.captures(line) {
            let group_name = caps.get(1).unwrap().as_str().trim();
            debug!("Found group: {}", group_name);
            current_group = Some(group_name.to_string());
            
            if !inventory.get_group(group_name).is_some() {
                debug!("Creating group: {}", group_name);
                let group = HostGroup::new(group_name);
                inventory.add_group(group);
            }
            
            continue;
        }
        
        // If we're in a vars section, process variables
        if let Some(group_ref) = &current_group {
            if group_ref.ends_with(":vars") {
                if let Some(caps) = var_line_re.captures(line) {
                    let var_name = caps.get(1).unwrap().as_str().trim();
                    let var_value = caps.get(2).unwrap().as_str().trim();
                    let group_name = &group_ref[0..group_ref.len() - 5]; // Remove ":vars" suffix
                    
                    debug!("Setting group variable for {}: {} = {}", group_name, var_name, var_value);
                    
                    if let Some(group) = inventory.groups.get_mut(group_name) {
                        group.set_variable(var_name, var_value);
                    }
                    
                    continue;
                }
            } else if group_ref.ends_with(":children") {
                // 处理:children部分中的子组
                let child_name = line.trim();
                let parent_name = &group_ref[0..group_ref.len() - 9]; // Remove ":children" suffix
                
                debug!("Adding child group {} to parent {}", child_name, parent_name);
                
                // 如果子组不存在，先创建
                if !inventory.get_group(child_name).is_some() {
                    debug!("Creating child group: {}", child_name);
                    let child_group = HostGroup::new(child_name).with_parent(parent_name);
                    inventory.add_group(child_group);
                } else {
                    // 如果子组已存在，更新其父组
                    if let Some(child_group) = inventory.groups.get_mut(child_name) {
                        child_group.parent = Some(parent_name.to_string());
                    }
                }
                
                // 在父组中添加子组
                if let Some(parent_group) = inventory.groups.get_mut(parent_name) {
                    parent_group.add_child(child_name);
                }
                
                continue;
            }
        }
        
        // Process normal host entry with potential variables
        // Split the line into host part and the rest
        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        let host_entry = parts[0].trim();
        
        let mut host_name = host_entry;
        let mut port = 22;
        
        // Check if host has a port specification
        if let Some(caps) = host_port_re.captures(host_entry) {
            host_name = caps.get(1).unwrap().as_str().trim();
            port = caps.get(2).unwrap().as_str().parse::<u16>().unwrap_or(22);
            debug!("Host with port specification: {} (port: {})", host_name, port);
        }
        
        // Add host if it doesn't exist
        if !inventory.hosts.contains_key(host_name) {
            debug!("Adding new host: {}", host_name);
            let host = Host::new(host_name)
                .with_port(port);
            inventory.add_host(host);
        }
        
        // Add host to current group
        if let Some(group_name) = &current_group {
            if !group_name.ends_with(":vars") {
                if let Some(group) = inventory.groups.get_mut(group_name) {
                    debug!("Adding host {} to group {}", host_name, group_name);
                    group.add_host(host_name);
                }
            }
        } else {
            // If no group specified, add to "ungrouped"
            if let Some(group) = inventory.groups.get_mut("ungrouped") {
                debug!("Adding host {} to group ungrouped", host_name);
                group.add_host(host_name);
            }
        }
        
        // Process variables on the line (if any)
        if parts.len() > 1 {
            let vars_part = parts[1].trim();
            debug!("Processing variables for host {}: {}", host_name, vars_part);
            
            // 识别并处理以空格分隔的多个变量
            let mut var_start = 0;
            let mut in_quote = false;
            let mut quote_char = ' ';
            let mut var_name = "";
            
            for (i, c) in vars_part.char_indices() {
                if c == '=' && var_name.is_empty() {
                    var_name = vars_part[var_start..i].trim();
                    var_start = i + 1;
                } else if (c == ' ' || i == vars_part.len() - 1) && !in_quote && !var_name.is_empty() {
                    // 如果是最后一个字符，包括它
                    let var_value_end = if i == vars_part.len() - 1 { i + 1 } else { i };
                    let var_value = vars_part[var_start..var_value_end].trim();
                    
                    debug!("Extracted variable: {} = {}", var_name, var_value);
                    
                    // 清除引号
                    let var_value = if (var_value.starts_with('\'') && var_value.ends_with('\'')) || 
                                       (var_value.starts_with('"') && var_value.ends_with('"')) {
                        &var_value[1..var_value.len()-1]
                    } else {
                        var_value
                    };
                    
                    if let Some(host) = inventory.hosts.get_mut(host_name) {
                        debug!("Setting variable for host {}: {} = {}", host_name, var_name, var_value);
                        host.set_variable(var_name, var_value);
                    }
                    
                    var_name = "";
                    var_start = i + 1;
                } else if (c == '\'' || c == '"') && (!in_quote || c == quote_char) {
                    in_quote = !in_quote;
                    if in_quote {
                        quote_char = c;
                    }
                }
            }
            
            // 简单的空格分隔的变量处理（备用方法）
            if var_name.is_empty() {
                for var_entry in vars_part.split_whitespace() {
                    if let Some((name, value)) = var_entry.split_once('=') {
                        let var_name = name.trim();
                        let mut var_value = value.trim();
                        
                        // 清除引号
                        if (var_value.starts_with('\'') && var_value.ends_with('\'')) || 
                           (var_value.starts_with('"') && var_value.ends_with('"')) {
                            var_value = &var_value[1..var_value.len()-1];
                        }
                        
                        if let Some(host) = inventory.hosts.get_mut(host_name) {
                            debug!("Setting variable for host {}: {} = {}", host_name, var_name, var_value);
                            host.set_variable(var_name, var_value);
                        }
                    }
                }
            }
        }
    }
    
    // Make sure all hosts are also added to the "all" group
    let all_hosts: Vec<String> = inventory.hosts.keys().cloned().collect();
    if let Some(all_group) = inventory.groups.get_mut("all") {
        for host_name in all_hosts {
            all_group.add_host(&host_name);
        }
    }
    
    // Log inventory summary
    info!("Inventory parsed: {} hosts, {} groups", inventory.hosts.len(), inventory.groups.len());
    for (name, group) in &inventory.groups {
        info!("Group '{}': {} hosts", name, group.hosts.len());
    }
    
    // Apply group variables so hosts inherit group-level vars
    inventory.apply_group_vars();
    Ok(inventory)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_inventory_parsing() {
        // 创建临时清单文件
        let mut temp_file = NamedTempFile::new().unwrap();
        write!(temp_file, r#"
# 测试清单文件
[webservers]
test1.example.com ansible_ssh_user=admin
test2.example.com:2222 ansible_ssh_user=user

[dbservers]
db1.example.com

[webservers:vars]
http_port=80
https_port=443

[all:vars]
ansible_ssh_pass=testpassword
        "#).unwrap();

        // 解析并验证清单内容
        let inventory = parse_inventory(temp_file.path().to_str().unwrap()).unwrap();
        
        // 验证主机和组
        assert_eq!(inventory.hosts.len(), 3);
        assert_eq!(inventory.groups.len(), 4); // all, ungrouped, webservers, dbservers
        
        // 验证组成员
        let webservers = inventory.groups.get("webservers").unwrap();
        assert_eq!(webservers.hosts.len(), 2);
        
        // 验证变量
        let test1 = inventory.hosts.get("test1.example.com").unwrap();
        assert_eq!(test1.get_variable("ansible_ssh_user").unwrap(), "admin");
        
        // 验证 all:vars 被继承
        assert_eq!(test1.get_variable("ansible_ssh_pass").unwrap(), "testpassword");
        
        // 验证端口设置
        let test2 = inventory.hosts.get("test2.example.com").unwrap();
        assert_eq!(test2.port, 2222);
    }
} 