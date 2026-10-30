[English / 简体中文](../../zh-CN/guides/tasks.md) · [Index / 目录](../index.md)

# Task controls and selection

<a id="purpose"></a>

## Before you start

when selects execution; changed_when/failed_when override the result conditions. Lists of conditions use AND. until updates register on every attempt; retries counts additional tries, delay defaults to 5 seconds. timeout limits each target attempt and terminates descendants. Uncertain/unreachable outcomes stop retries.

<a id="usage"></a>

## Usage

```bash
rustsible playbook examples/selection/playbook.yml -i examples/quickstart/inventory.yml --tags deploy --list-tasks
```

<a id="behavior"></a>

## Behavior and boundaries

loop/with_items accept lists or typed expressions; loop_control supports loop_var and index_var. A loop register contains per-item results. Play/task environment is per-request. Static tags inherit; dynamic include tags select the include itself, while apply propagates settings. never tasks need explicit selection; notified handlers are not filtered by task tags.

<a id="reference"></a>

## Reference

See the [module index](../reference/modules.md), [configuration](../reference/configuration.md), [keywords](../reference/keywords.md) and [results](../reference/results.md).
