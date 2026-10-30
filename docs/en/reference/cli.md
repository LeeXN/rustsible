[English / 简体中文](../../zh-CN/reference/cli.md) · [Index / 目录](../index.md)

# Command line reference

Generated from Clap. Defaults shown here are parser defaults; configuration and connection variables can override applicable defaults. Use the configuration and variable references for effective precedence. Commands/identifiers remain untranslated. Dry-run permits temporary worker staging but forbids business mutation; syntax/list are controller-only.

<a id="rustsible"></a>

## `rustsible`

An Ansible-inspired automation tool written in Rust

```text
Usage: rustsible [OPTIONS] <COMMAND>
```

| Argument | Required | Default | Meaning |
|---|---|---|---|
| `--vault-password-file` | false | — | Read a Vault password from a regular non-executable file |
| `--vault-id` | false | — | Vault password source: ID@FILE or ID@prompt |
| `--ask-vault-pass` | false | false | Prompt for a Vault password on the controlling terminal |
| `--config` | false | — | Explicit ansible.cfg file |
| `--worker-dir` | false | — | Directory containing the release worker manifest and binaries |
| `-h, --help` | false | — | Print help |
| `-V, --version` | false | — | Print version |

<a id="rustsible-build-info"></a>

## `rustsible build-info`

Print release build identity as JSON without loading configuration

```text
Usage: rustsible build-info [OPTIONS]
```

| Argument | Required | Default | Meaning |
|---|---|---|---|
| `--config` | false | — | Explicit ansible.cfg file |
| `--worker-dir` | false | — | Directory containing the release worker manifest and binaries |
| `-h, --help` | false | — | Print help |

<a id="rustsible-vault"></a>

## `rustsible vault`

Encrypt, decrypt, view or rekey Ansible Vault files

```text
Usage: rustsible vault [OPTIONS] <COMMAND>
```

| Argument | Required | Default | Meaning |
|---|---|---|---|
| `--vault-password-file` | false | — | Read a Vault password from a regular non-executable file |
| `--vault-id` | false | — | Vault password source: ID@FILE or ID@prompt |
| `--ask-vault-pass` | false | false | Prompt for a Vault password on the controlling terminal |
| `--config` | false | — | Explicit ansible.cfg file |
| `--worker-dir` | false | — | Directory containing the release worker manifest and binaries |
| `-h, --help` | false | — | Print help |

<a id="rustsible-vault-encrypt"></a>

## `rustsible vault encrypt`



```text
Usage: rustsible vault encrypt [OPTIONS] <FILE>
```

| Argument | Required | Default | Meaning |
|---|---|---|---|
| `--vault-password-file` | false | — | Read a Vault password from a regular non-executable file |
| `--vault-id` | false | — | Vault password source: ID@FILE or ID@prompt |
| `--ask-vault-pass` | false | false | Prompt for a Vault password on the controlling terminal |
| `input` | true | — | Vault input file, or - for standard input |
| `--output` | false | — | Output file; '-' writes stdout (default: replace input) |
| `--encrypt-vault-id` | false | — | Choose the encryption label and matching password source |
| `--config` | false | — | Explicit ansible.cfg file |
| `--worker-dir` | false | — | Directory containing the release worker manifest and binaries |
| `-h, --help` | false | — | Print help |

<a id="rustsible-vault-decrypt"></a>

## `rustsible vault decrypt`



```text
Usage: rustsible vault decrypt [OPTIONS] <FILE>
```

| Argument | Required | Default | Meaning |
|---|---|---|---|
| `--vault-password-file` | false | — | Read a Vault password from a regular non-executable file |
| `--vault-id` | false | — | Vault password source: ID@FILE or ID@prompt |
| `--ask-vault-pass` | false | false | Prompt for a Vault password on the controlling terminal |
| `input` | true | — | Vault input file, or - for standard input |
| `--output` | false | — | Output file; '-' writes stdout (default: replace input) |
| `--config` | false | — | Explicit ansible.cfg file |
| `--worker-dir` | false | — | Directory containing the release worker manifest and binaries |
| `-h, --help` | false | — | Print help |

<a id="rustsible-vault-view"></a>

## `rustsible vault view`



```text
Usage: rustsible vault view [OPTIONS] <FILE>
```

| Argument | Required | Default | Meaning |
|---|---|---|---|
| `--vault-password-file` | false | — | Read a Vault password from a regular non-executable file |
| `--vault-id` | false | — | Vault password source: ID@FILE or ID@prompt |
| `--ask-vault-pass` | false | false | Prompt for a Vault password on the controlling terminal |
| `input` | true | — | Vault input file, or - for standard input |
| `--config` | false | — | Explicit ansible.cfg file |
| `--worker-dir` | false | — | Directory containing the release worker manifest and binaries |
| `-h, --help` | false | — | Print help |

<a id="rustsible-vault-rekey"></a>

## `rustsible vault rekey`



```text
Usage: rustsible vault rekey [OPTIONS] <FILE>
```

| Argument | Required | Default | Meaning |
|---|---|---|---|
| `--vault-password-file` | false | — | Read a Vault password from a regular non-executable file |
| `--vault-id` | false | — | Vault password source: ID@FILE or ID@prompt |
| `--ask-vault-pass` | false | false | Prompt for a Vault password on the controlling terminal |
| `input` | true | — | Vault input file, or - for standard input |
| `--output` | false | — | Output file; '-' writes stdout (default: replace input) |
| `--encrypt-vault-id` | false | — | Choose the encryption label and matching password source |
| `--new-vault-password-file` | false | — | New password file for rekey; repeatable |
| `--new-vault-id` | false | — | New rekey password source: ID@FILE or ID@prompt; repeatable |
| `--config` | false | — | Explicit ansible.cfg file |
| `--worker-dir` | false | — | Directory containing the release worker manifest and binaries |
| `-h, --help` | false | — | Print help |

<a id="rustsible-collection"></a>

## `rustsible collection`

Package, verify or install a native collection

```text
Usage: rustsible collection [OPTIONS] <COMMAND>
```

| Argument | Required | Default | Meaning |
|---|---|---|---|
| `--config` | false | — | Explicit ansible.cfg file |
| `--worker-dir` | false | — | Directory containing the release worker manifest and binaries |
| `-h, --help` | false | — | Print help |

<a id="rustsible-collection-pack"></a>

## `rustsible collection pack`



```text
Usage: rustsible collection pack [OPTIONS] --output <output> <input>
```

| Argument | Required | Default | Meaning |
|---|---|---|---|
| `input` | true | — | Collection source directory |
| `--output` | true | — | New collection archive path; existing files are not overwritten |
| `--config` | false | — | Explicit ansible.cfg file |
| `--worker-dir` | false | — | Directory containing the release worker manifest and binaries |
| `-h, --help` | false | — | Print help |

<a id="rustsible-collection-verify"></a>

## `rustsible collection verify`



```text
Usage: rustsible collection verify [OPTIONS] <input>
```

| Argument | Required | Default | Meaning |
|---|---|---|---|
| `input` | true | — | Collection archive to verify or install |
| `--config` | false | — | Explicit ansible.cfg file |
| `--worker-dir` | false | — | Directory containing the release worker manifest and binaries |
| `-h, --help` | false | — | Print help |

<a id="rustsible-collection-install"></a>

## `rustsible collection install`



```text
Usage: rustsible collection install [OPTIONS] <input>
```

| Argument | Required | Default | Meaning |
|---|---|---|---|
| `input` | true | — | Collection archive to verify or install |
| `--path` | false | — | Collection installation root; custom roots also need search-path configuration |
| `--config` | false | — | Explicit ansible.cfg file |
| `--worker-dir` | false | — | Directory containing the release worker manifest and binaries |
| `-h, --help` | false | — | Print help |

<a id="rustsible-playbook"></a>

## `rustsible playbook`

Run a Rustsible playbook

```text
Usage: rustsible playbook [OPTIONS] <playbook>
```

| Argument | Required | Default | Meaning |
|---|---|---|---|
| `--collections-path` | false | — | Explicit collection root; authorizes its declared native executables |
| `--vault-password-file` | false | — | Read a Vault password from a regular non-executable file |
| `--vault-id` | false | — | Vault password source: ID@FILE or ID@prompt |
| `--ask-vault-pass` | false | false | Prompt for a Vault password on the controlling terminal |
| `-e, --extra-vars` | false | — | Extra variables as name=value, a YAML/JSON mapping, or @file |
| `-k, --ask-pass` | false | false | Prompt for SSH password on the controlling terminal |
| `-K, --ask-become-pass` | false | false | Prompt for privilege escalation password on the controlling terminal |
| `--private-key` | false | — | SSH private key file |
| `--diff` | false | false | Show bounded structured differences from supported modules |
| `--tags` | false | — | Run tasks matching comma-separated tags |
| `--skip-tags` | false | — | Skip tasks matching comma-separated tags |
| `--syntax-check` | false | false | Validate static playbook structure without connecting |
| `--list-hosts` | false | false | List selected hosts as JSON without connecting |
| `--list-tasks` | false | false | List selected static tasks as JSON without connecting |
| `playbook` | true | — | Playbook file to run |
| `-i, --inventory` | false | — | Specify inventory file path (default: 'inventory') |
| `-v` | false | 0 | Increase verbosity (up to -vvvv) |
| `-l, --limit` | false | — | Limit to specified hosts or groups |
| `--check` | false | false | Perform a dry run without making changes |
| `-f, --forks` | false | 5 | Maximum number of hosts processed in parallel |
| `--config` | false | — | Explicit ansible.cfg file |
| `--worker-dir` | false | — | Directory containing the release worker manifest and binaries |
| `-h, --help` | false | — | Print help |

<a id="rustsible-ad-hoc"></a>

## `rustsible ad-hoc`

Run an ad-hoc command on managed nodes

```text
Usage: rustsible ad-hoc [OPTIONS] --module <MODULE> <pattern>
```

| Argument | Required | Default | Meaning |
|---|---|---|---|
| `--collections-path` | false | — | Explicit collection root; authorizes its declared native executables |
| `--vault-password-file` | false | — | Read a Vault password from a regular non-executable file |
| `--vault-id` | false | — | Vault password source: ID@FILE or ID@prompt |
| `--ask-vault-pass` | false | false | Prompt for a Vault password on the controlling terminal |
| `-e, --extra-vars` | false | — | Extra variables as name=value, a YAML/JSON mapping, or @file |
| `-k, --ask-pass` | false | false | Prompt for SSH password on the controlling terminal |
| `-K, --ask-become-pass` | false | false | Prompt for privilege escalation password on the controlling terminal |
| `--private-key` | false | — | SSH private key file |
| `--diff` | false | false | Show bounded structured differences from supported modules |
| `pattern` | true | — | Host pattern to target |
| `-m, --module` | true | — | Module name to execute |
| `-a, --args` | false | — | Module arguments |
| `-i, --inventory` | false | — | Specify inventory file path (default: 'inventory') |
| `-v` | false | 0 | Increase verbosity (up to -vvvv) |
| `-b, --become` | false | false | Run operations with privilege escalation |
| `--become-user` | false | — | Run operations as this user (default: root) |
| `-B, --background` | false | 0 | Maximum runtime for a native background task; zero runs synchronously |
| `-P, --poll` | false | 15 | Poll interval for a background task; zero returns its job ID immediately |
| `--check` | false | false | Predict changes without applying them |
| `-f, --forks` | false | 5 | Maximum number of hosts processed in parallel |
| `--config` | false | — | Explicit ansible.cfg file |
| `--worker-dir` | false | — | Directory containing the release worker manifest and binaries |
| `-h, --help` | false | — | Print help |

<a id="rustsible-inventory-debug"></a>

## `rustsible inventory-debug`

Debug and display information about inventory files

```text
Usage: rustsible inventory-debug [OPTIONS]
```

| Argument | Required | Default | Meaning |
|---|---|---|---|
| `--collections-path` | false | — | Explicit collection root; authorizes its declared native executables |
| `--vault-password-file` | false | — | Read a Vault password from a regular non-executable file |
| `--vault-id` | false | — | Vault password source: ID@FILE or ID@prompt |
| `--ask-vault-pass` | false | false | Prompt for a Vault password on the controlling terminal |
| `-i, --inventory` | false | — | Specify inventory file path (default: 'inventory') |
| `-v` | false | 0 | Increase verbosity (up to -vvvv) |
| `--config` | false | — | Explicit ansible.cfg file |
| `--worker-dir` | false | — | Directory containing the release worker manifest and binaries |
| `-h, --help` | false | — | Print help |
