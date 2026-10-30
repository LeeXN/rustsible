[English / 简体中文](../../zh-CN/guides/collections.md) · [Index / 目录](../index.md)

# Install and use native collections

<a id="purpose"></a>

## Before you start

Declarative roles/playbooks can be discovered in project collections; repository executables are not automatically authorized. Native execution requires explicit installation or --collections-path/configuration. Default installations also have an authorization record outside the package contents.

<a id="usage"></a>

## Usage

```bash
rustsible collection verify example.native-0.1.0.tar.gz
rustsible collection install example.native-0.1.0.tar.gz
rustsible playbook example.native.inspect -i inventory.yml
```

<a id="behavior"></a>

## Behavior and boundaries

Use namespace.collection.component FQCNs. Search explicit roots first, project collections next, installed home collections last. collections_path/ANSIBLE_COLLECTIONS_PATH provide configured roots; custom install --path also needs a configured search path. Packages verify ELF, architecture, checksums and all files before atomic installation. Python plugins, links, unsafe ownership and tampering are rejected.

<a id="reference"></a>

## Reference

See the [module index](../reference/modules.md), [configuration](../reference/configuration.md), [keywords](../reference/keywords.md) and [results](../reference/results.md).
