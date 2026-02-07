pub mod host;
mod parser;

use anyhow::Result;
pub use host::{Host, HostGroup};
use log::{debug, info, warn};
use std::collections::{HashMap, HashSet};

pub struct Inventory {
    pub hosts: HashMap<String, Host>,
    pub groups: HashMap<String, HostGroup>,
}

impl Inventory {
    pub fn new() -> Self {
        let mut groups = HashMap::new();
        groups.insert("all".to_string(), HostGroup::new("all"));
        groups.insert("ungrouped".to_string(), HostGroup::new("ungrouped"));

        Inventory {
            hosts: HashMap::new(),
            groups,
        }
    }

    pub fn add_host(&mut self, host: Host) {
        let name = host.name.clone();
        self.hosts.insert(name, host);
    }

    pub fn add_group(&mut self, group: HostGroup) {
        let name = group.name.clone();
        self.groups.insert(name, group);
    }

    pub fn get_group(&self, name: &str) -> Option<&HostGroup> {
        self.groups.get(name)
    }

    pub fn filter_hosts(&self, pattern: &str) -> Vec<Host> {
        // Simple pattern matching for now (exact group or host names)
        // In a full implementation, we would handle complex patterns with wildcards

        let mut result = HashSet::new();

        info!("Filtering hosts with pattern: {}", pattern);
        debug!(
            "Available groups: {:?}",
            self.groups.keys().collect::<Vec<_>>()
        );

        // 如果匹配一个组，添加该组的所有主机（包括子组的主机）
        if let Some(group) = self.groups.get(pattern) {
            info!("Found group '{}' with {} hosts", pattern, group.hosts.len());

            // 获取直接主机成员
            for host_name in &group.hosts {
                if let Some(_host) = self.hosts.get(host_name) {
                    debug!("Adding host '{}' from group '{}'", host_name, pattern);
                    result.insert(host_name.clone());
                } else {
                    warn!(
                        "Host '{}' referenced in group '{}' not found in inventory",
                        host_name, pattern
                    );
                }
            }

            // 获取所有子组的主机
            for child_name in &group.children {
                self.add_group_hosts_recursive(child_name, &mut result);
            }
        } else {
            debug!("No group found with name '{}'", pattern);

            // 如果匹配一个主机，直接添加
            if let Some(_host) = self.hosts.get(pattern) {
                debug!("Adding host '{}' by direct name match", pattern);
                result.insert(pattern.to_string());
            } else {
                warn!("No host or group found matching '{}'", pattern);
            }
        }

        // 在返回主机列表前应用变量继承
        // 创建可变副本以应用变量
        let mut inventory_clone = Inventory::new();
        for (name, host) in &self.hosts {
            if result.contains(name) {
                inventory_clone.hosts.insert(name.clone(), host.clone());
            }
        }
        for (name, group) in &self.groups {
            inventory_clone.groups.insert(name.clone(), group.clone());
        }

        // 应用组变量继承
        inventory_clone.apply_group_vars();

        // 转换为向量，但使用处理过变量的主机，返回克隆而非引用
        let hosts: Vec<Host> = result
            .iter()
            .filter_map(|name| inventory_clone.hosts.get(name).cloned())
            .collect();

        info!("Found {} hosts matching pattern '{}'", hosts.len(), pattern);

        // If debug is enabled, list all matched hosts
        if log::log_enabled!(log::Level::Debug) {
            for host in &hosts {
                debug!(
                    "Matched host: {} ({}:{})",
                    host.name, host.hostname, host.port
                );
                debug!("Host variables: {:?}", host.variables);
                debug!("Host inherited variables: {:?}", host.inherited_variables);
            }
        }

        hosts
    }

    /// 递归地添加一个组及其子组的所有主机
    fn add_group_hosts_recursive(&self, group_name: &str, result: &mut HashSet<String>) {
        if let Some(group) = self.groups.get(group_name) {
            debug!(
                "Processing child group '{}' with {} hosts",
                group_name,
                group.hosts.len()
            );

            // 添加直接主机成员
            for host_name in &group.hosts {
                if let Some(_host) = self.hosts.get(host_name) {
                    debug!(
                        "Adding host '{}' from child group '{}'",
                        host_name, group_name
                    );
                    result.insert(host_name.clone());
                }
            }

            // 递归处理子组
            for child_name in &group.children {
                self.add_group_hosts_recursive(child_name, result);
            }
        }
    }

    /// 将组变量应用到组内的主机
    fn apply_group_vars(&mut self) {
        debug!("Applying group variables to hosts");

        // 首先应用 all 组的变量
        if let Some(all_group) = self.groups.get("all") {
            debug!("Applying variables from 'all' group to all hosts");
            for (name, host) in &mut self.hosts {
                for (key, value) in &all_group.variables {
                    if host.add_inherited_variable(key, value) {
                        debug!(
                            "Set inherited variable from 'all' for host {}: {} = {}",
                            name, key, value
                        );
                    }
                }
            }
        }

        // 收集所有组和它们的变量
        let mut group_vars: HashMap<String, HashMap<String, String>> = HashMap::new();
        for (group_name, group) in &self.groups {
            if !group.variables.is_empty() {
                group_vars.insert(group_name.clone(), group.variables.clone());
            }
        }

        // 为每个组应用变量
        for (group_name, group) in &self.groups {
            if group_vars.contains_key(group_name) && group_name != "all" {
                debug!(
                    "Applying variables from group '{}' to its hosts",
                    group_name
                );

                // 获取组内所有主机
                let host_names: Vec<String> = group.hosts.iter().cloned().collect();

                // 应用到组内的主机
                for host_name in host_names {
                    if let Some(host) = self.hosts.get_mut(&host_name) {
                        let vars = &group_vars[group_name];
                        for (key, value) in vars {
                            // 只有在主机没有定义该变量时才应用组变量
                            if host.add_inherited_variable(key, value) {
                                debug!(
                                    "Set inherited variable for host {}: {} = {}",
                                    host_name, key, value
                                );
                            }
                        }
                    }
                }
            }
        }

        // 处理子组继承
        self.apply_parent_group_vars();
    }

    /// 处理子组从父组继承变量
    fn apply_parent_group_vars(&mut self) {
        debug!("Applying parent group variables to child groups");

        // 创建组关系的副本以避免借用冲突
        let mut parent_child_map: HashMap<String, Vec<String>> = HashMap::new();

        for (group_name, group) in &self.groups {
            if let Some(parent) = &group.parent {
                parent_child_map
                    .entry(parent.clone())
                    .or_insert_with(Vec::new)
                    .push(group_name.clone());
            }
        }

        // 递归应用父组变量到子组
        for (parent_name, children) in &parent_child_map {
            if let Some(parent) = self.groups.get(parent_name) {
                let parent_vars = parent.variables.clone();

                for child_name in children {
                    if let Some(child) = self.groups.get_mut(child_name) {
                        for (key, value) in &parent_vars {
                            if !child.variables.contains_key(key) {
                                debug!(
                                    "Setting inherited variable for group {}: {} = {}",
                                    child_name, key, value
                                );
                                child.add_variable(key, value);
                            }
                        }
                    }
                }
            }
        }
    }
}

pub fn parse(inventory_path: &str) -> Result<Inventory> {
    parser::parse_inventory(inventory_path)
}
