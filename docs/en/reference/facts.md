[English / 简体中文](../../zh-CN/reference/facts.md) · [Index / 目录](../index.md)

# Facts and target information

<a id="collection"></a>

## Collection and filtering

Facts are gathered by default on each selected target. setup accepts gather_subset (default all), filter (top-level shell patterns) and gather_timeout (1–604800s, default10). Explicit setup merges into the same host state. Disabling facts does not invent controller-time or target information.

<a id="subsets"></a>

## Supported subsets

platform: architecture/system/kernel/kernel_version/machine/hostname/nodename/fqdn/domain. distribution: distribution/version/major_version/release/os_family. date_time: date/time components, epoch and ISO8601. network: interfaces/all_ipv4_addresses/all_ipv6_addresses/default_ipv4. min includes platform/distribution/date_time; all adds network. Individual supported selectors are accepted. !all retains min; !all,!min starts empty.

<a id="access"></a>

## Access and omissions

Setup result names use ansible_ prefixes; ansible_facts drops that prefix. For example ansible_architecture and ansible_facts.architecture refer to the collected architecture. set_fact uses user-defined names. There is no persistent cache, Python collector, arbitrary executable local facts, or the full Ansible collector set. Delegated facts follow delegate_facts.
