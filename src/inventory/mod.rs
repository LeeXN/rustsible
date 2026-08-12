pub mod host;
mod parser;

use anyhow::{bail, Result};
pub use host::{Host, HostGroup};
use log::{debug, info, warn};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct Inventory {
    pub hosts: HashMap<String, Host>,
    pub groups: HashMap<String, HostGroup>,
}

impl Default for Inventory {
    fn default() -> Self {
        Self::new()
    }
}

impl Inventory {
    pub fn new() -> Self {
        let mut groups = HashMap::new();
        groups.insert("all".to_string(), HostGroup::new("all"));
        groups.insert("ungrouped".to_string(), HostGroup::new("ungrouped"));

        Self {
            hosts: HashMap::new(),
            groups,
        }
    }

    pub fn add_host(&mut self, host: Host) {
        self.hosts.insert(host.name.clone(), host);
    }

    pub fn add_group(&mut self, group: HostGroup) {
        self.groups.insert(group.name.clone(), group);
    }

    pub fn get_group(&self, name: &str) -> Option<&HostGroup> {
        self.groups.get(name)
    }

    /// Filter hosts using the commonly used Ansible pattern operators.
    ///
    /// `,` and `:` form unions, `&` intersects, and `!` excludes hosts.  As in
    /// Ansible, unions are combined first, then intersections, then
    /// exclusions, regardless of their textual order. Terms may be host
    /// names, group names, `all`, or globs containing `*` and `?`. If there is
    /// no union term, intersection/exclusion is evaluated against `all`.
    /// Returned hosts are always ordered by inventory name.
    pub fn filter_hosts(&self, pattern: &str) -> Vec<Host> {
        info!("Filtering hosts with pattern: {}", pattern);

        let terms: Vec<&str> = pattern
            .split([',', ':'])
            .map(str::trim)
            .filter(|term| !term.is_empty())
            .collect();
        if terms.is_empty() {
            return Vec::new();
        }

        let all: HashSet<String> = self.hosts.keys().cloned().collect();
        let mut selected = HashSet::new();
        let mut intersections = Vec::new();
        let mut exclusions = HashSet::new();
        let mut has_positive_term = false;

        for term in terms {
            let (operator, operand) = match term.as_bytes().first() {
                Some(b'&') => ('&', term[1..].trim()),
                Some(b'!') => ('!', term[1..].trim()),
                _ => ('+', term),
            };
            if operand.is_empty() {
                continue;
            }

            let matches = self.expand_pattern_term(operand);
            match operator {
                '+' => {
                    selected.extend(matches);
                    has_positive_term = true;
                }
                '&' => intersections.push(matches),
                '!' => exclusions.extend(matches),
                _ => unreachable!(),
            }
        }

        if !has_positive_term {
            selected = all;
        }
        for intersection in intersections {
            selected.retain(|name| intersection.contains(name));
        }
        selected.retain(|name| !exclusions.contains(name));

        let mut inventory = self.clone();
        inventory.apply_group_vars();
        let mut names: Vec<String> = selected.into_iter().collect();
        names.sort();
        let hosts: Vec<Host> = names
            .into_iter()
            .filter_map(|name| inventory.hosts.get(&name).cloned())
            .collect();

        info!("Found {} hosts matching pattern '{}'", hosts.len(), pattern);
        if log::log_enabled!(log::Level::Debug) {
            for host in &hosts {
                debug!(
                    "Matched host: {} ({}:{}) with {} direct and {} inherited variable(s)",
                    host.name,
                    host.hostname,
                    host.port,
                    host.variables.len(),
                    host.inherited_variables.len()
                );
            }
        }
        hosts
    }

    fn expand_pattern_term(&self, term: &str) -> HashSet<String> {
        if term == "all" {
            return self.hosts.keys().cloned().collect();
        }

        let is_glob = term.contains(['*', '?']);
        let mut result = HashSet::new();

        if is_glob {
            let mut host_names: Vec<&String> = self.hosts.keys().collect();
            host_names.sort();
            for name in host_names {
                if glob_matches(term, name) {
                    result.insert(name.clone());
                }
            }

            let mut group_names: Vec<&String> = self.groups.keys().collect();
            group_names.sort();
            for group_name in group_names {
                if glob_matches(term, group_name) {
                    self.collect_group_hosts(group_name, &mut result, &mut HashSet::new());
                }
            }
        } else if self.hosts.contains_key(term) {
            result.insert(term.to_string());
        } else if self.groups.contains_key(term) {
            self.collect_group_hosts(term, &mut result, &mut HashSet::new());
        } else {
            warn!("No host or group found matching '{}'", term);
        }

        result
    }

    /// Collect a group's direct and descendant hosts.  `visited` makes this
    /// safe even for an Inventory constructed programmatically with a cycle.
    fn collect_group_hosts(
        &self,
        group_name: &str,
        result: &mut HashSet<String>,
        visited: &mut HashSet<String>,
    ) {
        if !visited.insert(group_name.to_string()) {
            warn!("Ignoring cyclic group reference at '{}'", group_name);
            return;
        }
        let Some(group) = self.groups.get(group_name) else {
            return;
        };

        let mut direct_hosts: Vec<&String> = group.hosts.iter().collect();
        direct_hosts.sort();
        for host_name in direct_hosts {
            if self.hosts.contains_key(host_name) {
                result.insert(host_name.clone());
            } else {
                warn!(
                    "Host '{}' referenced in group '{}' is absent from the inventory",
                    host_name, group_name
                );
            }
        }

        let mut children: Vec<&String> = group.children.iter().collect();
        children.sort();
        for child in children {
            self.collect_group_hosts(child, result, visited);
        }
        visited.remove(group_name);
    }

    /// Validate the child graph and return an actionable error instead of
    /// allowing recursive consumers to overflow their stack.
    pub(crate) fn validate_group_cycles(&self) -> Result<()> {
        fn visit(
            inventory: &Inventory,
            group_name: &str,
            visiting: &mut Vec<String>,
            complete: &mut HashSet<String>,
        ) -> Result<()> {
            if let Some(position) = visiting.iter().position(|name| name == group_name) {
                let mut cycle = visiting[position..].to_vec();
                cycle.push(group_name.to_string());
                bail!("inventory group cycle detected: {}", cycle.join(" -> "));
            }
            if complete.contains(group_name) {
                return Ok(());
            }

            visiting.push(group_name.to_string());
            if let Some(group) = inventory.groups.get(group_name) {
                let mut children: Vec<&String> = group.children.iter().collect();
                children.sort();
                for child in children {
                    visit(inventory, child, visiting, complete)?;
                }
            }
            visiting.pop();
            complete.insert(group_name.to_string());
            Ok(())
        }

        let mut names: Vec<&String> = self.groups.keys().collect();
        names.sort();
        let mut complete = HashSet::new();
        for name in names {
            visit(self, name, &mut Vec::new(), &mut complete)?;
        }
        Ok(())
    }

    /// Apply group variables with deterministic precedence:
    /// `all < parent < child < host`.  Unrelated groups at the same depth are
    /// applied in lexical order (the lexically later group wins).
    pub(crate) fn apply_group_vars(&mut self) {
        for host in self.hosts.values_mut() {
            host.clear_inherited_variables();
        }

        if let Some(all_group) = self.groups.get("all") {
            let mut variables: Vec<_> = all_group.variables.iter().collect();
            variables.sort_by_key(|(key, _)| *key);
            let mut typed_variables: Vec<_> = all_group.typed_variables.iter().collect();
            typed_variables.sort_by_key(|(key, _)| *key);
            for host in self.hosts.values_mut() {
                for (key, value) in &variables {
                    host.add_inherited_variable(key, value);
                }
                for (key, value) in &typed_variables {
                    host.add_typed_inherited_variable(key, value);
                }
            }
        }

        let parents = self.parent_map();
        let mut memo = HashMap::new();
        let mut group_names: Vec<String> = self
            .groups
            .keys()
            .filter(|name| name.as_str() != "all")
            .cloned()
            .collect();
        group_names.sort_by(|left, right| {
            let left_depth = group_depth(left, &parents, &mut memo, &mut HashSet::new());
            let right_depth = group_depth(right, &parents, &mut memo, &mut HashSet::new());
            left_depth.cmp(&right_depth).then_with(|| left.cmp(right))
        });

        for group_name in group_names {
            let mut members = HashSet::new();
            self.collect_group_hosts(&group_name, &mut members, &mut HashSet::new());
            let Some(group) = self.groups.get(&group_name) else {
                continue;
            };
            let mut variables: Vec<_> = group.variables.iter().collect();
            variables.sort_by_key(|(key, _)| *key);
            let mut typed_variables: Vec<_> = group.typed_variables.iter().collect();
            typed_variables.sort_by_key(|(key, _)| *key);
            let mut members: Vec<String> = members.into_iter().collect();
            members.sort();
            for host_name in members {
                if let Some(host) = self.hosts.get_mut(&host_name) {
                    for (key, value) in &variables {
                        host.add_inherited_variable(key, value);
                    }
                    for (key, value) in &typed_variables {
                        host.add_typed_inherited_variable(key, value);
                    }
                }
            }
        }
    }

    fn parent_map(&self) -> HashMap<String, Vec<String>> {
        let mut parents: HashMap<String, Vec<String>> = HashMap::new();
        for (parent_name, group) in &self.groups {
            for child in &group.children {
                parents
                    .entry(child.clone())
                    .or_default()
                    .push(parent_name.clone());
            }
        }
        for values in parents.values_mut() {
            values.sort();
            values.dedup();
        }
        parents
    }
}

fn group_depth(
    group: &str,
    parents: &HashMap<String, Vec<String>>,
    memo: &mut HashMap<String, usize>,
    visiting: &mut HashSet<String>,
) -> usize {
    if group == "all" {
        return 0;
    }
    if let Some(depth) = memo.get(group) {
        return *depth;
    }
    if !visiting.insert(group.to_string()) {
        return 1;
    }
    let depth = parents
        .get(group)
        .map(|group_parents| {
            group_parents
                .iter()
                .map(|parent| group_depth(parent, parents, memo, visiting) + 1)
                .max()
                .unwrap_or(1)
        })
        .unwrap_or(1);
    visiting.remove(group);
    memo.insert(group.to_string(), depth);
    depth
}

fn glob_matches(pattern: &str, value: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let value: Vec<char> = value.chars().collect();
    let mut previous = vec![false; value.len() + 1];
    previous[0] = true;

    for part in pattern {
        let mut current = vec![false; value.len() + 1];
        if part == '*' {
            current[0] = previous[0];
            for index in 1..=value.len() {
                current[index] = previous[index] || current[index - 1];
            }
        } else {
            for index in 1..=value.len() {
                current[index] = previous[index - 1] && (part == '?' || part == value[index - 1]);
            }
        }
        previous = current;
    }
    previous[value.len()]
}

pub fn parse(inventory_path: &str) -> Result<Inventory> {
    parser::parse_inventory(inventory_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inventory_for_patterns() -> Inventory {
        let mut inventory = Inventory::new();
        for name in ["db1", "web1", "web2", "web-old"] {
            inventory.add_host(Host::new(name));
        }
        let mut web = HostGroup::new("web");
        web.add_host("web1");
        web.add_host("web2");
        web.add_host("web-old");
        inventory.add_group(web);
        let mut production = HostGroup::new("production");
        production.add_host("web1");
        production.add_host("db1");
        inventory.add_group(production);
        inventory
    }

    fn names(hosts: Vec<Host>) -> Vec<String> {
        hosts.into_iter().map(|host| host.name).collect()
    }

    #[test]
    fn host_patterns_support_union_intersection_exclusion_and_globs() {
        let inventory = inventory_for_patterns();
        assert_eq!(
            names(inventory.filter_hosts("all")),
            vec!["db1", "web-old", "web1", "web2"]
        );
        assert_eq!(
            names(inventory.filter_hosts("web*")),
            vec!["web-old", "web1", "web2"]
        );
        assert_eq!(
            names(inventory.filter_hosts("web:&production:!web-old")),
            vec!["web1"]
        );
        assert_eq!(
            names(inventory.filter_hosts("db1,web2")),
            vec!["db1", "web2"]
        );
    }

    #[test]
    fn cyclic_programmatic_groups_do_not_recurse_forever() {
        let mut inventory = inventory_for_patterns();
        inventory
            .groups
            .get_mut("web")
            .unwrap()
            .add_child("production");
        inventory
            .groups
            .get_mut("production")
            .unwrap()
            .add_child("web");
        assert!(inventory.validate_group_cycles().is_err());
        assert_eq!(
            names(inventory.filter_hosts("web")),
            vec!["db1", "web-old", "web1", "web2"]
        );
    }

    #[test]
    fn inheritance_is_deterministic_and_updates_connection_fields() {
        let mut inventory = Inventory::new();
        inventory.add_host(Host::new("node"));
        inventory
            .groups
            .get_mut("all")
            .unwrap()
            .set_variable("tier", "all");
        inventory
            .groups
            .get_mut("all")
            .unwrap()
            .set_variable("ansible_port", "2200");

        let mut parent = HostGroup::new("parent");
        parent.add_child("child");
        parent.set_variable("tier", "parent");
        parent.set_variable("ansible_host", "parent.example");
        inventory.add_group(parent);
        let mut child = HostGroup::new("child").with_parent("parent");
        child.add_host("node");
        child.set_variable("tier", "child");
        child.set_variable("ansible_port", "2222");
        inventory.add_group(child);

        inventory.apply_group_vars();
        let host = inventory.hosts.get("node").unwrap();
        assert_eq!(host.get_variable("tier").map(String::as_str), Some("child"));
        assert_eq!(host.hostname, "parent.example");
        assert_eq!(host.port, 2222);
    }
}
