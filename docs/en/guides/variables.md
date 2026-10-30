[English / 简体中文](../../zh-CN/guides/variables.md) · [Index / 目录](../index.md)

# Variables, templates and facts

<a id="purpose"></a>

## Before you start

Separate variable definitions from ordinary data. Definitions can reference other variables with cycle detection; command output, native inventory output and rendered strings are never compiled again. A single template expression preserves its type. Conditions must evaluate to booleans.

<a id="usage"></a>

## Usage

```bash
rustsible ad-hoc all -i examples/quickstart/inventory.yml -m debug -a '{"msg":"{{ greeting }}"}' -e greeting=hello
```

<a id="behavior"></a>

## Behavior and boundaries

Facts and registers survive across plays; play/task vars do not. setup runs on the target, including time. ansible_facts removes the ansible_ prefix used by setup results. Failed hosts remain excluded until explicitly cleared. hostvars sees persistent host state at task boundaries. Use default([]), default(omit), and an explicit stable salt for password_hash. See the variable and fact references.

<a id="reference"></a>

## Reference

See the [module index](../reference/modules.md), [configuration](../reference/configuration.md), [keywords](../reference/keywords.md) and [results](../reference/results.md).
