[English / 简体中文](../../zh-CN/tutorials/deployment.md) · [Index / 目录](../index.md)

# A deployment with roles and recovery

<a id="layout"></a>

## Files and responsibilities

Use examples/roles: main.yml statically imports deploy.yml; roles/web/tasks renders the role template; handlers records configuration changes; tasks/health.yml verifies the file; block/rescue/always demonstrates recovery. Only inventory and an entry playbook are mandatory. ansible.cfg, defaults, vars and handlers are added only when needed. Source-relative paths do not depend on the launch directory.

<a id="execute"></a>

## Run the example

```bash
demo_output=$(mktemp -d)
rustsible playbook examples/roles/main.yml -i examples/quickstart/inventory.yml -e "demo_output=$demo_output" --check
rustsible playbook examples/roles/main.yml -i examples/quickstart/inventory.yml -e "demo_output=$demo_output"
```

<a id="production"></a>

## From example to rollout

The deliberate /bin/false runs only outside check. This recovery tutorial is not an idempotence benchmark for arbitrary shell commands. For a three-host flow use examples/rolling with a fresh demo_root; serial [1,2] executes one host then two, run_once chooses a leader per batch, and delegated receipts remain attributed to the logical host. See [scheduling](../guides/scheduling.md). The complete download→Vault configuration→handler→health→byte-exact rollback scenario is maintained in the isolated release acceptance suite.
