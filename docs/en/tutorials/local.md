[English / 简体中文](../../zh-CN/tutorials/local.md) · [Index / 目录](../index.md)

# Your first local deployment

<a id="files"></a>

## Prepare the files

Run from the repository or extracted bundle root. The example inventory chooses local explicitly, playbook.yml creates an app directory and configuration, and cleanup.yml removes only that app directory. demo_root is mandatory and should be a newly created private directory. No sudo, system account or package operation is involved.

<a id="run"></a>

## Inspect, predict, apply, repeat

```bash
demo_root=$(mktemp -d)
rustsible playbook examples/quickstart/playbook.yml -i examples/quickstart/inventory.yml -e "demo_root=$demo_root" --syntax-check
rustsible playbook examples/quickstart/playbook.yml -i examples/quickstart/inventory.yml -e "demo_root=$demo_root" --check --diff
rustsible playbook examples/quickstart/playbook.yml -i examples/quickstart/inventory.yml -e "demo_root=$demo_root"
cat "$demo_root/app/config"
rustsible playbook examples/quickstart/playbook.yml -i examples/quickstart/inventory.yml -e "demo_root=$demo_root"
rustsible playbook examples/quickstart/cleanup.yml -i examples/quickstart/inventory.yml -e "demo_root=$demo_root"
```

<a id="observe"></a>

## Verify the target

Syntax check returns JSON and never connects. Check predicts file changes without creating app/. First apply writes application=rustsible and enabled=true, with mode 0600; the notified handler writes applied. Second apply reports changed=0. Cleanup removes app/; the caller-owned empty demo_root remains. Check may upload and remove a temporary worker; it is not a promise of zero temporary infrastructure writes.

<a id="next"></a>

## Understand the execution

Read [variables](../guides/variables.md), [file modules](../reference/modules/copy.md) and [roles](deployment.md). Use the matching language version of the same guide; executable blocks are identical.
