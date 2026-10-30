[English / 简体中文](../../zh-CN/guides/vault.md) · [Index / 目录](../index.md)

# Encrypted configuration

<a id="purpose"></a>

## Before you start

Vault supports AES256 1.1/1.2 files, !vault scalars, vault IDs and hidden prompts or regular non-executable password files. IDs guide key order rather than restricting which matching key can decrypt. Loaders decrypt in memory; intentional file deployment writes its requested business content.

<a id="usage"></a>

## Usage

```bash
rustsible vault encrypt vars.yml --vault-id prod@prompt
rustsible vault view vars.yml --vault-id prod@prompt
rustsible vault rekey vars.yml --vault-id prod@prompt --new-vault-password-file /secure/new-password
```

<a id="behavior"></a>

## Behavior and boundaries

Use --vault-password-file, repeated --vault-id ID@FILE or ID@prompt, or --ask-vault-pass. CLI decrypt/view/output - intentionally exposes plaintext: choose its destination deliberately. Password scripts are rejected. Used Vault input censors failure diagnostics and diff; still use no_log for tasks intentionally printing protected values. Unused encrypted scalars stay lazy; accessing one requires a working password.

<a id="reference"></a>

## Reference

See the [module index](../reference/modules.md), [configuration](../reference/configuration.md), [keywords](../reference/keywords.md) and [results](../reference/results.md).
