[English / 简体中文](../../zh-CN/guides/roles.md) · [Index / 目录](../index.md)

# Imports, roles and failure recovery

<a id="purpose"></a>

## Before you start

Static imports expand during loading; dynamic includes expand only when selected. Relative files follow the actual task/role source. Roles support tasks/defaults/vars/handlers/meta dependencies and selected *_from entry files. Import/dependency cycles are rejected.

<a id="usage"></a>

## Usage

```bash
rustsible playbook examples/roles/main.yml -i examples/quickstart/inventory.yml --list-tasks
```

<a id="behavior"></a>

## Behavior and boundaries

block failures may enter rescue, then always. Unreachable is not ordinary module failure and does not enter rescue. ignore_errors does not hide unreachable/uncertain results; ignore_unreachable is explicit. Notifications deduplicate and handlers run in definition order. pre_tasks/tasks/post_tasks establish flush phases; force_handlers and meta control flushing and host/play lifetime.

<a id="reference"></a>

## Reference

See the [module index](../reference/modules.md), [configuration](../reference/configuration.md), [keywords](../reference/keywords.md) and [results](../reference/results.md).
