[English / 简体中文](../../zh-CN/guides/scheduling.md) · [Index / 目录](../index.md)

# Rolling and delegated execution

<a id="purpose"></a>

## Before you start

linear keeps task barriers; free lets healthy hosts advance independently. forks caps active host controllers, throttle adds a task cap, serial partitions the original host set by count/percentage/list. run_once applies per serial batch.

<a id="usage"></a>

## Usage

```bash
rustsible playbook examples/rolling/playbook.yml -i examples/rolling/inventory -e demo_root=/tmp/reviewed-rollout --check
```

<a id="behavior"></a>

## Behavior and boundaries

delegate_to changes physical execution, not logical ownership of registers/notifications; delegate_facts explicitly assigns facts to the delegate. any_errors_fatal and strict max_fail_percentage stop later work. free rejects run_once, any_errors_fatal and max_fail_percentage combinations. fail_fast is a Rustsible extension. Use fresh, operator-owned demo_root values for actual runs.

<a id="reference"></a>

## Reference

See the [module index](../reference/modules.md), [configuration](../reference/configuration.md), [keywords](../reference/keywords.md) and [results](../reference/results.md).
