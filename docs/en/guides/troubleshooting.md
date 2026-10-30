[English / 简体中文](../../zh-CN/guides/troubleshooting.md) · [Index / 目录](../index.md)

# Troubleshooting by failure boundary

<a id="purpose"></a>

## Before you start

Start with build-info, inventory selection and syntax/list operations. Match controller/worker build IDs; inspect the effective connection values and exact execution identity before assuming a module bug. Do not replay a modifying task whose outcome is uncertain.

<a id="usage"></a>

## Usage

```bash
rustsible build-info
rustsible playbook site.yml -i inventory.yml --syntax-check
rustsible playbook site.yml -i inventory.yml --list-hosts
```

<a id="behavior"></a>

## Behavior and boundaries

Host-key mismatch: verify the host fingerprint. noexec/unsafe temporary parent: choose a safe executable rustsible_remote_tmp. Unknown parameter/filter: check this version’s reference, not all Ansible documentation. Permission failure: distinguish login, become and destination metadata. TLS failure: configure target CA/proxy rather than disabling verification. Missing SDK program: install/configure the collection and include the actual architecture. no_log/Vault censorship is deliberate; reproduce with nonsensitive fixtures rather than printing secrets.

<a id="reference"></a>

## Reference

See the [module index](../reference/modules.md), [configuration](../reference/configuration.md), [keywords](../reference/keywords.md) and [results](../reference/results.md).
