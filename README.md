# Rustsible

[English / 简体中文](README.zh-CN.md) · [MIT](LICENSE) · [CI](https://github.com/LeeXN/rustsible/actions/workflows/rust.yml)

Rustsible is a Linux automation tool written in Rust. The controller schedules work and temporarily deploys native workers over local execution or SSH. There is no persistent agent and no builtin Python/curl runtime dependency. Linux x86_64/aarch64 and the declared glibc/musl environments are supported.

Compatibility is the explicit tested subset of Ansible 2.21, not its entire ecosystem. The handbook covers 25 builtins, roles/recovery, rolling execution, async, Vault and controlled native collections with their exact boundaries. OS tools retain their own dependencies, such as OS Python used by DNF/Yum.

## Start here

- [Install, build and verify offline bundles](docs/en/tutorials/install.md)
- [First local deployment and idempotence](docs/en/tutorials/local.md)
- [First SSH deployment and identity](docs/en/tutorials/ssh.md)
- [Roles, handlers and recovery](docs/en/tutorials/deployment.md)
- [Complete user and developer handbook](docs/en/index.md)
- [25 modules and capabilities](docs/en/reference/modules.md)
- [CLI reference](docs/en/reference/cli.md)
- [Support matrix and safety differences](docs/en/reference/compatibility.md)
- [Native extension authoring](docs/en/development/native-extensions.md)
- [Contribution and validation](docs/en/development/contributing.md)

## Minimal local example

Prepare matching controller/workers using the install tutorial, then run from the source or extracted bundle root. The example operates only in the new directory.

```bash
demo_root=$(mktemp -d)
rustsible playbook examples/quickstart/playbook.yml -i examples/quickstart/inventory.yml -e "demo_root=$demo_root" --check --diff
rustsible playbook examples/quickstart/playbook.yml -i examples/quickstart/inventory.yml -e "demo_root=$demo_root"
rustsible playbook examples/quickstart/playbook.yml -i examples/quickstart/inventory.yml -e "demo_root=$demo_root"
```

Second execution should report changed=0; see the tutorial for exact contents and cleanup. Global --check cannot be canceled by task check_mode:false, but may stage and clean a temporary worker. no_log censors presentation rather than clearing registers. Uncertain modifications are never automatically replayed.

Offline bundles include both workers, the bilingual handbook, examples, RELEASE.json and recursive SHA256SUMS. Native collections require explicit installation/configuration to authorize execution; Python plugins never use a fallback. The handbook records actual platform/backend validation and evidence, separating native execution from cross-compilation.
