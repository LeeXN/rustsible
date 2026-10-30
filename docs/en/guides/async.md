[English / 简体中文](../../zh-CN/guides/async.md) · [Index / 目录](../index.md)

# Native background jobs

<a id="purpose"></a>

## Before you start

async sets a maximum lifetime; poll>0 waits and cleans finished state, poll:0 returns an opaque job ID for later async_status queries and explicit cleanup. SSH loss does not stop an accepted background job. Check never starts background work.

<a id="usage"></a>

## Usage

```bash
rustsible ad-hoc web -i inventory.yml -m command -a 'sleep 10' -B 30 -P 0
```

<a id="behavior"></a>

## Behavior and boundaries

State lives under the execution identity (default ~/.rustsible_async), with 0700 directories and 0600 files. ansible_async_dir selects a protected location. The supervisor retains required programs and kills descendants at TTL. Private results remain private on later queries. Supported modules declare async; controller file uploads are not background tasks.

<a id="reference"></a>

## Reference

See the [module index](../reference/modules.md), [configuration](../reference/configuration.md), [keywords](../reference/keywords.md) and [results](../reference/results.md).
