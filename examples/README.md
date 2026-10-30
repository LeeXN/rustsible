[English / 简体中文](README.zh-CN.md)

# Rustsible examples

Run from the repository or extracted bundle root. Both languages use the same configuration assets; actual execution requires a fresh private directory. Every example supports offline syntax inspection, which does not establish readiness of missing external prerequisites.

| Scenario | Behavior and prerequisites |
|---|---|
| [quickstart](quickstart/playbook.yml) | Local files, handler, exact second-run idempotence; demo_root required. |
| [inventory](inventory/hosts) | INI local group selection; no remote address or credential. |
| [variables](variables/playbook.yml) | Typed loops and facts across plays; existing demo_root. |
| [files](files/playbook.yml) | Template and line repair; demo_root required. |
| [roles](roles/main.yml) | Static import, role, handlers and intentional rescue; demo_output required. |
| [selection](selection/playbook.yml) | deploy/diagnostics tags; no target writes. |
| [rolling](rolling/playbook.yml) | Three logical local hosts; demo_root is a fresh path prefix. SSH requires your reviewed inventory. |
| [async](async/playbook.yml) | Native job launch/query/cleanup; fresh demo_root. |
| [assertions](assertions/playbook.yml) | Boolean conditions and deliberate fail/rescue; no business files. |
| [vault](vault/playbook.yml) | Prepared encrypted vault_variables with example_value; existing demo_root. |
| [http](http/playbook.yml) | Requires your isolated health_host/health_port/health_url service; no server installed automatically. |
| [artifacts](artifacts/playbook.yml) | Requires reviewed source_repo/release_revision/archive_source, Git on target, demo_root. |
| [native-collection](native-collection/README.md) | Explicitly build/install the sample native collection; see its README. |

## Local start

```bash
demo_root=$(mktemp -d)
rustsible playbook examples/quickstart/playbook.yml -i examples/quickstart/inventory.yml -e "demo_root=$demo_root" --check --diff
rustsible playbook examples/quickstart/playbook.yml -i examples/quickstart/inventory.yml -e "demo_root=$demo_root"
rustsible playbook examples/quickstart/playbook.yml -i examples/quickstart/inventory.yml -e "demo_root=$demo_root"
rustsible playbook examples/quickstart/cleanup.yml -i examples/quickstart/inventory.yml -e "demo_root=$demo_root"
```

Other state modules have complete playbooks in the [module reference](../docs/en/reference/modules.md). Account/package/service recipes require disposable targets. Historical “all features” material is removed; minimal rejected-input regressions live in tests/fixtures, not public tutorials. Rust helper programs are development/test tools, excluded from offline user examples.
