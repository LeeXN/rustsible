[English / 简体中文](../../zh-CN/guides/inventory.md) · [Index / 目录](../index.md)

# Inventory and host selection

<a id="purpose"></a>

## Before you start

INI and YAML inventories support hosts, groups and child groups. Use explicit ansible_connection=local for local examples. all, glob, union, intersection (&) and exclusion (!) select hosts; --limit restricts the result. Hosts outside the play may still appear in hostvars.

<a id="usage"></a>

## Usage

```bash
rustsible playbook examples/quickstart/playbook.yml -i examples/quickstart/inventory.yml --list-hosts
```

<a id="behavior"></a>

## Behavior and boundaries

Inventory names are logical identities, not necessarily DNS names. ansible_host selects the address. group_vars/host_vars are loaded beside inventory, then beside playbook; host variables outrank group variables. Cycles and invalid connection values fail explicitly.

<a id="reference"></a>

## Reference

See the [module index](../reference/modules.md), [configuration](../reference/configuration.md), [keywords](../reference/keywords.md) and [results](../reference/results.md).
