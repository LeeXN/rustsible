[English / 简体中文](../../zh-CN/reference/variables.md) · [Index / 目录](../index.md)

# Variable sources and precedence

<a id="layers"></a>

## Normal variable layers

From lower to higher: configuration baseline; role defaults; inventory groups then host; facts; play vars; vars_files; role vars; task vars; include_vars; set_fact/register; include/role parameters; extra vars. Role defaults are below inventory; role vars join their documented role scope. Aliases normalize inside each layer. Task keyword connection defaults are below connection variables. Loop bindings/current-attempt register are temporary execution overlays.

<a id="sources"></a>

## Loading and lifetimes

group_vars/host_vars load beside inventory, then playbook, from extensionless/YAML/JSON files or sorted directories. vars_files supports ordered fallback candidates and source-relative paths; fact-dependent paths wait until facts exist. include_vars executes when selected. Facts/registers persist across plays; play/task locals do not. Extra vars accept repeated key=value, structured JSON/YAML, or @file.

<a id="magic"></a>

## Runtime names

hostvars, groups, group_names, inventory_hostname, inventory_file, inventory_dir, playbook_dir, ansible_config_file, ansible_play_hosts_all and ansible_play_batch describe the current scope. Role context exposes role_path, short role_name, FQCN ansible_role_name and ansible_collection_name. ansible_check_mode/ansible_diff_mode reflect invocation flags and cannot be spoofed by normal variables; task check_mode:true does not rewrite that global flag.

<a id="types"></a>

## Template and data boundaries

MiniJinja expressions preserve native booleans/lists/numbers/strings. Mixed text remains a string. Definitions resolve referenced names with cycle detection; data from commands and native providers remains literal. No string-to-YAML guessing or repeat rendering. when/until/assert require booleans, not arbitrary truthy strings. Use default(omit) to remove a parameter and password_hash with an explicit stable salt.
