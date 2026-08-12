use anyhow::{bail, Context, Result};
use log::{debug, info};
use serde_yaml::Value;
use std::fs;
use std::path::Path;

use super::{Host, HostGroup, Inventory};

#[derive(Debug, Clone)]
enum IniSection {
    Hosts(String),
    Vars(String),
    Children(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InventoryFormat {
    Ini,
    Yaml,
}

pub fn parse_inventory(inventory_path: &str) -> Result<Inventory> {
    let path = Path::new(inventory_path);
    if !path.exists() {
        bail!("Inventory file not found: {}", inventory_path);
    }

    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read inventory file {}", inventory_path))?;
    let format = detect_format(path, &content);
    info!(
        "Parsing {} inventory file: {}",
        match format {
            InventoryFormat::Ini => "INI",
            InventoryFormat::Yaml => "YAML",
        },
        inventory_path
    );

    let mut inventory = match format {
        InventoryFormat::Ini => parse_ini(&content)?,
        InventoryFormat::Yaml => parse_yaml(&content)?,
    };
    finish_inventory(&mut inventory)?;

    info!(
        "Inventory parsed: {} hosts, {} groups",
        inventory.hosts.len(),
        inventory.groups.len()
    );
    Ok(inventory)
}

fn detect_format(path: &Path, content: &str) -> InventoryFormat {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("yaml" | "yml") => InventoryFormat::Yaml,
        Some("ini") => InventoryFormat::Ini,
        _ if looks_like_yaml(content) => InventoryFormat::Yaml,
        _ => InventoryFormat::Ini,
    }
}

fn looks_like_yaml(content: &str) -> bool {
    let Some(line) = content
        .lines()
        .map(str::trim_end)
        .find(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
    else {
        return false;
    };
    let trimmed = line.trim_start();
    if trimmed.starts_with("---") || trimmed.starts_with('{') || trimmed.starts_with("- ") {
        return true;
    }
    if trimmed.starts_with('[') {
        return false;
    }
    trimmed.find(':').is_some_and(|colon| {
        let key = trimmed[..colon].trim();
        !key.is_empty()
            && !key.chars().any(char::is_whitespace)
            && trimmed[colon + 1..]
                .chars()
                .next()
                .is_none_or(|character| character.is_whitespace() || matches!(character, '{' | '['))
    })
}

fn parse_ini(content: &str) -> Result<Inventory> {
    let mut inventory = Inventory::new();
    let mut section = None;

    for (line_index, raw_line) in content.lines().enumerate() {
        let line_number = line_index + 1;
        let fields = tokenize_ini_line(raw_line)
            .with_context(|| format!("invalid INI inventory syntax on line {}", line_number))?;
        if fields.is_empty() {
            continue;
        }
        debug!("Processing non-empty INI inventory line {}", line_number);

        if fields.len() == 1 && fields[0].starts_with('[') && fields[0].ends_with(']') {
            let header = &fields[0][1..fields[0].len() - 1];
            if header.is_empty() {
                bail!("empty inventory group name on line {}", line_number);
            }
            let (group_name, kind) = match header.rsplit_once(':') {
                Some((name, "vars")) => (name, "vars"),
                Some((name, "children")) => (name, "children"),
                Some((_, suffix)) => bail!(
                    "unknown inventory section suffix '{}' on line {}; expected 'vars' or 'children'",
                    suffix,
                    line_number
                ),
                _ => (header, "hosts"),
            };
            validate_group_name(group_name, line_number)?;
            ensure_group(&mut inventory, group_name);
            section = Some(match kind {
                "vars" => IniSection::Vars(group_name.to_string()),
                "children" => IniSection::Children(group_name.to_string()),
                _ => IniSection::Hosts(group_name.to_string()),
            });
            continue;
        }
        if fields[0].starts_with('[') {
            bail!("malformed inventory section header on line {}", line_number);
        }

        match &section {
            Some(IniSection::Vars(group_name)) => {
                if fields.len() != 1 {
                    bail!(
                        "group variable on line {} must be one key=value assignment; quote values containing spaces",
                        line_number
                    );
                }
                let (key, value) = parse_assignment(&fields[0], line_number)?;
                validate_connection_variable(key, value, line_number)?;
                ensure_group(&mut inventory, group_name).set_variable(key, value);
            }
            Some(IniSection::Children(parent_name)) => {
                if fields.len() != 1 || fields[0].contains('=') {
                    bail!("invalid child group reference on line {}", line_number);
                }
                validate_group_name(&fields[0], line_number)?;
                link_groups(&mut inventory, parent_name, &fields[0]);
            }
            Some(IniSection::Hosts(group_name)) => {
                parse_ini_host_line(&mut inventory, Some(group_name), &fields, line_number)?;
            }
            None => parse_ini_host_line(&mut inventory, None, &fields, line_number)?,
        }
    }
    Ok(inventory)
}

fn parse_ini_host_line(
    inventory: &mut Inventory,
    group_name: Option<&str>,
    fields: &[String],
    line_number: usize,
) -> Result<()> {
    let host_spec = fields.first().ok_or_else(|| {
        anyhow::anyhow!(
            "inventory host entry on line {} does not contain a host name",
            line_number
        )
    })?;
    let (host_name, literal_port) = parse_host_spec(host_spec)
        .with_context(|| format!("invalid host on inventory line {}", line_number))?;
    ensure_host(inventory, &host_name);
    if let Some(port) = literal_port {
        ensure_host(inventory, &host_name).set_variable("ansible_port", &port.to_string());
    }

    if let Some(group_name) = group_name {
        ensure_group(inventory, group_name).add_host(&host_name);
    }

    for assignment in &fields[1..] {
        let (key, value) = parse_assignment(assignment, line_number)?;
        validate_connection_variable(key, value, line_number)?;
        ensure_host(inventory, &host_name).set_variable(key, value);
    }
    Ok(())
}

/// Split an INI host line without losing spaces inside quotes. Quotes are
/// removed and backslash escapes the following character both inside and
/// outside quotes. Comments start at `#` or `;` only between fields.
fn tokenize_ini_line(line: &str) -> Result<Vec<String>> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;

    for character in line.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if character == active_quote {
                quote = None;
            } else {
                current.push(character);
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            '#' | ';' if current.is_empty() => break,
            character if character.is_whitespace() => {
                if !current.is_empty() {
                    fields.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(character),
        }
    }

    if escaped {
        bail!("line ends with an incomplete escape");
    }
    if quote.is_some() {
        bail!("unterminated quoted value");
    }
    if !current.is_empty() {
        fields.push(current);
    }
    Ok(fields)
}

fn parse_assignment(field: &str, line_number: usize) -> Result<(&str, &str)> {
    let Some((key, value)) = field.split_once('=') else {
        bail!("expected key=value on inventory line {}", line_number);
    };
    if key.is_empty() {
        bail!("empty variable name on inventory line {}", line_number);
    }
    Ok((key, value))
}

fn parse_yaml(content: &str) -> Result<Inventory> {
    let root: Value = serde_yaml::from_str(content).context("failed to parse YAML inventory")?;
    let mut inventory = Inventory::new();
    match root {
        Value::Null => return Ok(inventory),
        Value::Mapping(groups) => {
            for (group_name, definition) in groups {
                let group_name = yaml_key(&group_name, "top-level group")?;
                parse_yaml_group(&mut inventory, group_name, &definition, None)?;
            }
        }
        _ => bail!("YAML inventory root must be a mapping of group names"),
    }
    Ok(inventory)
}

fn parse_yaml_group(
    inventory: &mut Inventory,
    group_name: &str,
    definition: &Value,
    parent: Option<&str>,
) -> Result<()> {
    if group_name.is_empty() {
        bail!("YAML inventory contains an empty group name");
    }
    ensure_group(inventory, group_name);
    if let Some(parent) = parent {
        link_groups(inventory, parent, group_name);
    }

    let entries = match definition {
        Value::Null => return Ok(()),
        Value::Mapping(entries) => entries,
        _ => bail!("YAML inventory group '{}' must be a mapping", group_name),
    };

    for (key, value) in entries {
        match yaml_key(key, &format!("group '{}' field", group_name))? {
            "hosts" => parse_yaml_hosts(inventory, group_name, value)?,
            "vars" => parse_yaml_group_vars(inventory, group_name, value)?,
            "children" => parse_yaml_children(inventory, group_name, value)?,
            unknown => bail!(
                "unknown field '{}' in YAML inventory group '{}'; expected hosts, vars, or children",
                unknown,
                group_name
            ),
        }
    }
    Ok(())
}

fn parse_yaml_hosts(inventory: &mut Inventory, group_name: &str, value: &Value) -> Result<()> {
    match value {
        Value::Null => Ok(()),
        Value::Mapping(hosts) => {
            for (host, variables) in hosts {
                let host = yaml_key(host, &format!("host in group '{}'", group_name))?;
                add_yaml_host(inventory, group_name, host, variables)?;
            }
            Ok(())
        }
        Value::Sequence(hosts) => {
            for host in hosts {
                match host {
                    Value::String(host) => {
                        add_yaml_host(inventory, group_name, host, &Value::Null)?
                    }
                    Value::Mapping(entry) if entry.len() == 1 => {
                        let Some((host, variables)) = entry.iter().next() else {
                            bail!(
                                "host entry in YAML inventory group '{}' cannot be empty",
                                group_name
                            );
                        };
                        let host = yaml_key(host, &format!("host in group '{}'", group_name))?;
                        add_yaml_host(inventory, group_name, host, variables)?;
                    }
                    _ => bail!(
                        "hosts in YAML inventory group '{}' must be a mapping or a list of host names",
                        group_name
                    ),
                }
            }
            Ok(())
        }
        _ => bail!(
            "'hosts' in YAML inventory group '{}' must be a mapping or sequence",
            group_name
        ),
    }
}

fn add_yaml_host(
    inventory: &mut Inventory,
    group_name: &str,
    host_spec: &str,
    variables: &Value,
) -> Result<()> {
    let (host_name, literal_port) = parse_host_spec(host_spec)
        .with_context(|| format!("invalid YAML inventory host '{}'", host_spec))?;
    ensure_host(inventory, &host_name);
    ensure_group(inventory, group_name).add_host(&host_name);

    if let Some(port) = literal_port {
        ensure_host(inventory, &host_name).set_variable("ansible_port", &port.to_string());
    }
    match variables {
        Value::Null => Ok(()),
        Value::Mapping(variables) => {
            for (key, value) in variables {
                let key = yaml_key(key, &format!("variables for host '{}'", host_name))?;
                validate_yaml_connection_variable(key, value)
                    .with_context(|| format!("invalid variable for YAML host '{}'", host_name))?;
                let text = yaml_variable_value(value)?;
                validate_connection_variable(key, &text, 0)
                    .with_context(|| format!("invalid variable for YAML host '{}'", host_name))?;
                ensure_host(inventory, &host_name).set_typed_variable(key, value.clone());
            }
            Ok(())
        }
        _ => bail!("variables for YAML host '{}' must be a mapping", host_name),
    }
}

fn parse_yaml_group_vars(inventory: &mut Inventory, group_name: &str, value: &Value) -> Result<()> {
    let Value::Mapping(variables) = value else {
        bail!(
            "'vars' in YAML inventory group '{}' must be a mapping",
            group_name
        );
    };
    for (key, value) in variables {
        let key = yaml_key(key, &format!("variables for group '{}'", group_name))?;
        validate_yaml_connection_variable(key, value)
            .with_context(|| format!("invalid variable for YAML group '{}'", group_name))?;
        let text = yaml_variable_value(value)?;
        validate_connection_variable(key, &text, 0)
            .with_context(|| format!("invalid variable for YAML group '{}'", group_name))?;
        ensure_group(inventory, group_name).set_typed_variable(key, value.clone());
    }
    Ok(())
}

fn parse_yaml_children(inventory: &mut Inventory, parent_name: &str, value: &Value) -> Result<()> {
    match value {
        Value::Null => Ok(()),
        Value::Mapping(children) => {
            for (child, definition) in children {
                let child = yaml_key(child, &format!("children of group '{}'", parent_name))?;
                parse_yaml_group(inventory, child, definition, Some(parent_name))?;
            }
            Ok(())
        }
        Value::Sequence(children) => {
            for child in children {
                let Value::String(child) = child else {
                    bail!(
                        "children of YAML group '{}' must be group names",
                        parent_name
                    );
                };
                link_groups(inventory, parent_name, child);
            }
            Ok(())
        }
        _ => bail!(
            "'children' in YAML inventory group '{}' must be a mapping or sequence",
            parent_name
        ),
    }
}

fn yaml_key<'a>(value: &'a Value, context: &str) -> Result<&'a str> {
    match value {
        Value::String(value) if !value.is_empty() => Ok(value),
        _ => bail!("{} name must be a non-empty string", context),
    }
}

fn yaml_variable_value(value: &Value) -> Result<String> {
    match value {
        Value::Null => Ok(String::new()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Number(value) => Ok(value.to_string()),
        Value::String(value) => Ok(value.clone()),
        Value::Sequence(_) | Value::Mapping(_) | Value::Tagged(_) => serde_yaml::to_string(value)
            .map(|value| value.trim().to_string())
            .context("failed to convert YAML inventory variable to text"),
    }
}

fn parse_host_spec(specification: &str) -> Result<(String, Option<u16>)> {
    if specification.is_empty() {
        bail!("host name cannot be empty");
    }
    if let Some(ipv6) = specification.strip_prefix('[') {
        let Some(closing_bracket) = ipv6.find(']') else {
            bail!(
                "unterminated bracketed IPv6 host '{}': missing ']'",
                specification
            );
        };
        let host = &ipv6[..closing_bracket];
        let suffix = &ipv6[closing_bracket + 1..];
        if host.is_empty() {
            bail!("host name cannot be empty");
        }
        return match suffix {
            "" => Ok((host.to_string(), None)),
            suffix if suffix.starts_with(':') => {
                Ok((host.to_string(), Some(parse_port(&suffix[1..])?)))
            }
            _ => bail!("unexpected text after bracketed host '{}'", specification),
        };
    }

    if specification.matches(':').count() == 1 {
        let Some((host, port)) = specification.rsplit_once(':') else {
            bail!("invalid host and port specification '{}'", specification);
        };
        if host.is_empty() {
            bail!("host name cannot be empty");
        }
        return Ok((host.to_string(), Some(parse_port(port)?)));
    }
    Ok((specification.to_string(), None))
}

fn parse_port(value: &str) -> Result<u16> {
    let port = value
        .parse::<u16>()
        .with_context(|| format!("invalid SSH port '{}': expected 1..=65535", value))?;
    if port == 0 {
        bail!("invalid SSH port '0': expected 1..=65535");
    }
    Ok(port)
}

fn validate_connection_variable(key: &str, value: &str, line_number: usize) -> Result<()> {
    if matches!(key, "ansible_port" | "ansible_ssh_port") {
        parse_port(value).with_context(|| {
            if line_number == 0 {
                format!("invalid value for {}", key)
            } else {
                format!("invalid value for {} on line {}", key, line_number)
            }
        })?;
    }
    if matches!(key, "ansible_become" | "ansible_sudo")
        && !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "yes" | "true" | "on" | "0" | "no" | "false" | "off"
        )
    {
        if line_number == 0 {
            bail!(
                "invalid value for {}: expected true/false, yes/no, on/off, or 1/0",
                key
            );
        }
        bail!(
            "invalid value for {} on line {}: expected true/false, yes/no, on/off, or 1/0",
            key,
            line_number
        );
    }
    if matches!(
        key,
        "rustsible_host_key_checking"
            | "ansible_host_key_checking"
            | "ansible_ssh_host_key_checking"
    ) && !matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "yes" | "true" | "on" | "0" | "no" | "false" | "off"
    ) {
        bail!("invalid boolean value for {}", key);
    }
    if matches!(
        key,
        "ansible_ssh_timeout"
            | "ansible_timeout"
            | "rustsible_command_timeout"
            | "ansible_command_timeout"
    ) && value.parse::<u64>().map_or(true, |seconds| seconds == 0)
    {
        bail!(
            "invalid timeout value for {}: expected a positive integer",
            key
        );
    }
    if matches!(key, "ansible_connection" | "ansible_connection_type")
        && !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "local" | "ssh" | "smart" | "paramiko"
        )
    {
        bail!("unsupported connection type for {}", key);
    }
    Ok(())
}

fn validate_yaml_connection_variable(key: &str, value: &Value) -> Result<()> {
    let string_variables = [
        "ansible_host",
        "ansible_ssh_host",
        "ansible_user",
        "ansible_ssh_user",
        "ansible_password",
        "ansible_ssh_pass",
        "ansible_become_password",
        "ansible_become_pass",
        "ansible_sudo_pass",
        "ansible_ssh_sudo_pass",
        "ansible_become_user",
        "ansible_sudo_user",
        "ansible_ssh_private_key_file",
        "ansible_connection",
        "ansible_connection_type",
        "rustsible_known_hosts_file",
        "ansible_ssh_known_hosts_file",
    ];
    if string_variables.contains(&key) {
        return match value {
            Value::String(value) if !value.trim().is_empty() => Ok(()),
            _ => bail!("{} must be a non-empty string", key),
        };
    }

    let boolean_variables = [
        "ansible_become",
        "ansible_sudo",
        "rustsible_host_key_checking",
        "ansible_host_key_checking",
        "ansible_ssh_host_key_checking",
    ];
    if boolean_variables.contains(&key) {
        return match value {
            Value::Bool(_) => Ok(()),
            Value::String(value)
                if matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "yes" | "true" | "on" | "0" | "no" | "false" | "off"
                ) =>
            {
                Ok(())
            }
            _ => bail!("{} must be a boolean", key),
        };
    }

    let integer_variables = [
        "ansible_port",
        "ansible_ssh_port",
        "ansible_ssh_timeout",
        "ansible_timeout",
        "rustsible_command_timeout",
        "ansible_command_timeout",
    ];
    if integer_variables.contains(&key) {
        let valid = match value {
            Value::Number(number) => number.as_u64().is_some_and(|number| number > 0),
            Value::String(value) => value.parse::<u64>().is_ok_and(|number| number > 0),
            _ => false,
        };
        if !valid {
            bail!("{} must be a positive integer", key);
        }
    }
    Ok(())
}

fn validate_group_name(name: &str, line_number: usize) -> Result<()> {
    if name.is_empty() || name.chars().any(char::is_whitespace) {
        bail!(
            "invalid inventory group name '{}' on line {}",
            name,
            line_number
        );
    }
    Ok(())
}

fn ensure_host<'a>(inventory: &'a mut Inventory, host_name: &str) -> &'a mut Host {
    inventory
        .hosts
        .entry(host_name.to_string())
        .or_insert_with(|| Host::new(host_name))
}

fn ensure_group<'a>(inventory: &'a mut Inventory, group_name: &str) -> &'a mut HostGroup {
    inventory
        .groups
        .entry(group_name.to_string())
        .or_insert_with(|| HostGroup::new(group_name))
}

fn link_groups(inventory: &mut Inventory, parent_name: &str, child_name: &str) {
    ensure_group(inventory, parent_name).add_child(child_name);

    // `children` is authoritative and supports multiple parents. Keep the
    // legacy single-parent field as a deterministic compatibility hint.
    let child = ensure_group(inventory, child_name);
    if child
        .parent
        .as_ref()
        .is_none_or(|current| parent_name < current.as_str())
    {
        child.parent = Some(parent_name.to_string());
    }
}

fn finish_inventory(inventory: &mut Inventory) -> Result<()> {
    inventory.validate_group_cycles()?;

    let mut host_names: Vec<String> = inventory.hosts.keys().cloned().collect();
    host_names.sort();
    let grouped: std::collections::HashSet<String> = inventory
        .groups
        .iter()
        .filter(|(name, _)| name.as_str() != "all" && name.as_str() != "ungrouped")
        .flat_map(|(_, group)| group.hosts.iter().cloned())
        .collect();

    let all = ensure_group(inventory, "all");
    all.hosts.clear();
    for host_name in &host_names {
        all.add_host(host_name);
    }
    let ungrouped = ensure_group(inventory, "ungrouped");
    ungrouped.hosts.clear();
    for host_name in host_names {
        if !grouped.contains(&host_name) {
            ungrouped.add_host(&host_name);
        }
    }

    inventory.apply_group_vars();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::{Builder, NamedTempFile};

    #[test]
    fn empty_ini_host_entry_returns_a_diagnostic_error() {
        let mut inventory = Inventory::new();
        let error = parse_ini_host_line(&mut inventory, None, &[], 7).unwrap_err();
        assert!(error.to_string().contains("line 7"));
        assert!(error.to_string().contains("host name"));
    }

    #[test]
    fn inventory_finalization_safely_restores_builtin_groups() {
        let mut inventory = Inventory::new();
        inventory.groups.remove("all");
        inventory.groups.remove("ungrouped");
        inventory.add_host(Host::new("standalone"));

        finish_inventory(&mut inventory).unwrap();

        assert!(inventory.groups["all"].hosts.contains("standalone"));
        assert!(inventory.groups["ungrouped"].hosts.contains("standalone"));
    }

    #[test]
    fn parses_ini_hosts_variables_and_inheritance() {
        let mut file = NamedTempFile::new().unwrap();
        write!(
            file,
            r#"
[webservers]
test1.example.com ansible_ssh_user=admin greeting="hello world" escaped=hello\ world
test2.example.com:2222 ansible_ssh_user=user

[webservers:vars]
http_port=80

[all:vars]
ansible_ssh_pass='test password'
"#
        )
        .unwrap();

        let inventory = parse_inventory(file.path().to_str().unwrap()).unwrap();
        assert_eq!(inventory.hosts.len(), 2);
        let test1 = inventory.hosts.get("test1.example.com").unwrap();
        assert_eq!(
            test1.get_variable("greeting").map(String::as_str),
            Some("hello world")
        );
        assert_eq!(
            test1.get_variable("escaped").map(String::as_str),
            Some("hello world")
        );
        assert_eq!(
            test1.get_variable("ansible_ssh_pass").map(String::as_str),
            Some("test password")
        );
        assert_eq!(inventory.hosts.get("test2.example.com").unwrap().port, 2222);
    }

    #[test]
    fn rejects_invalid_ini_ports_and_group_cycles() {
        let mut bad_port = NamedTempFile::new().unwrap();
        writeln!(bad_port, "node:not-a-port").unwrap();
        assert!(parse_inventory(bad_port.path().to_str().unwrap()).is_err());

        let mut bad_variable = NamedTempFile::new().unwrap();
        writeln!(bad_variable, "node ansible_port=70000").unwrap();
        assert!(parse_inventory(bad_variable.path().to_str().unwrap()).is_err());

        let mut bad_become = NamedTempFile::new().unwrap();
        writeln!(bad_become, "node ansible_become=tru").unwrap();
        let error = parse_inventory(bad_become.path().to_str().unwrap()).unwrap_err();
        assert!(error.to_string().contains("ansible_become"));

        let mut bad_section = NamedTempFile::new().unwrap();
        writeln!(bad_section, "[web:childen]").unwrap();
        let error = parse_inventory(bad_section.path().to_str().unwrap()).unwrap_err();
        assert!(error
            .to_string()
            .contains("unknown inventory section suffix"));

        let mut cycle = NamedTempFile::new().unwrap();
        write!(cycle, "[one:children]\ntwo\n[two:children]\none\n").unwrap();
        let error = parse_inventory(cycle.path().to_str().unwrap()).unwrap_err();
        assert!(error.to_string().contains("cycle"));
    }

    #[test]
    fn parses_nested_yaml_inventory_and_connection_variables() {
        let mut file = Builder::new().suffix(".yaml").tempfile().unwrap();
        write!(
            file,
            r#"
all:
  vars:
    ansible_user: deploy
  children:
    production:
      vars:
        environment: production
        ansible_host: gateway.example.com
      children:
        web:
          vars:
            ansible_port: 2202
          hosts:
            web1:
              greeting: "hello world"
              enabled: true
              ports: [80, 443]
            web2:
"#
        )
        .unwrap();

        let inventory = parse_inventory(file.path().to_str().unwrap()).unwrap();
        let web1 = inventory.hosts.get("web1").unwrap();
        assert_eq!(web1.hostname, "gateway.example.com");
        assert_eq!(web1.port, 2202);
        assert_eq!(
            web1.get_variable("ansible_user").map(String::as_str),
            Some("deploy")
        );
        assert_eq!(
            web1.get_variable("environment").map(String::as_str),
            Some("production")
        );
        assert_eq!(
            web1.get_variable("greeting").map(String::as_str),
            Some("hello world")
        );
        assert_eq!(
            web1.typed_variables.get("enabled"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            web1.typed_variables.get("ports"),
            Some(&Value::Sequence(vec![80.into(), 443.into()]))
        );
        assert_eq!(inventory.filter_hosts("production").len(), 2);
    }

    #[test]
    fn parses_repository_yaml_fixture_without_pseudo_hosts() {
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/inventory/simple.yaml");
        let inventory = parse_inventory(fixture.to_str().unwrap()).unwrap();

        let mut names: Vec<&str> = inventory.hosts.keys().map(String::as_str).collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "db1.example.com",
                "db2.example.com",
                "localhost",
                "web1.example.com",
                "web2.example.com"
            ]
        );
        assert_eq!(inventory.hosts["db1.example.com"].port, 2222);
        assert_eq!(
            inventory.hosts["localhost"]
                .get_variable("ansible_connection")
                .map(String::as_str),
            Some("local")
        );
    }

    #[test]
    fn detects_yaml_by_content_and_rejects_invalid_yaml_schema() {
        let mut yaml_without_extension = NamedTempFile::new().unwrap();
        write!(yaml_without_extension, "all:\n  hosts:\n    localhost:\n").unwrap();
        let inventory = parse_inventory(yaml_without_extension.path().to_str().unwrap()).unwrap();
        assert!(inventory.hosts.contains_key("localhost"));

        let mut invalid_yaml = Builder::new().suffix(".yaml").tempfile().unwrap();
        write!(invalid_yaml, "all: [unterminated").unwrap();
        let error = parse_inventory(invalid_yaml.path().to_str().unwrap()).unwrap_err();
        assert!(error.to_string().contains("YAML"));

        let mut invalid_schema = Builder::new().suffix(".yml").tempfile().unwrap();
        write!(invalid_schema, "all:\n  unexpected: value\n").unwrap();
        let error = parse_inventory(invalid_schema.path().to_str().unwrap()).unwrap_err();
        assert!(error.to_string().contains("unknown field"));

        let mut invalid_become = Builder::new().suffix(".yaml").tempfile().unwrap();
        write!(
            invalid_become,
            "all:\n  hosts:\n    localhost:\n      ansible_become: sometimes\n"
        )
        .unwrap();
        let error = parse_inventory(invalid_become.path().to_str().unwrap()).unwrap_err();
        assert!(format!("{error:#}").contains("ansible_become"));

        for variable in [
            "ansible_user: [root]",
            "ansible_host: {name: localhost}",
            "ansible_ssh_private_key_file: true",
            "ansible_timeout: zero",
        ] {
            let mut invalid = Builder::new().suffix(".yaml").tempfile().unwrap();
            write!(
                invalid,
                "all:\n  hosts:\n    localhost:\n      {variable}\n"
            )
            .unwrap();
            assert!(parse_inventory(invalid.path().to_str().unwrap()).is_err());
        }
    }
}
