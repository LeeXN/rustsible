[English / 简体中文](../../zh-CN/guides/connections.md) · [Index / 目录](../index.md)

# Connections, credentials and become

<a id="purpose"></a>

## Before you start

Connection values are resolved from the same final typed context used by task arguments. They can override keyword/default settings. SSH reuses a session for an effective host/identity configuration; reset_connection or changed connection settings creates a new lease.

<a id="usage"></a>

## Usage

```bash
rustsible ad-hoc web -i inventory.yml -m command -a 'id -un' --ask-pass --ask-become-pass -b
```

<a id="behavior"></a>

## Behavior and boundaries

Builtin become supports sudo. Configure ansible_become, ansible_become_user and ansible_become_password; passwords travel through protected input, not process arguments. Remote workers use 0700 directories, request files 0600. A noexec parent requires a different rustsible_remote_tmp. Host keys are checked strictly; an absent credential in a noninteractive session is an error.

<a id="reference"></a>

## Reference

See the [module index](../reference/modules.md), [configuration](../reference/configuration.md), [keywords](../reference/keywords.md) and [results](../reference/results.md).
