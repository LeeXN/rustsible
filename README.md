# Rustsible

[![CI](https://github.com/LeeXN/rustsible/actions/workflows/rust.yml/badge.svg)](https://github.com/LeeXN/rustsible/actions/workflows/rust.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

Rustsible is an experimental, Ansible-inspired configuration and command runner written in Rust.

> Rustsible is not a drop-in replacement for Ansible. It implements a useful subset of Ansible's inventory, playbook, templating, and module behavior, but compatibility is incomplete. Review a playbook with `--check` and test it in a disposable environment before using it on important systems.

## Current scope

Rustsible currently provides:

- INI and YAML inventory parsing, group variables, child groups, and host-pattern filtering;
- playbook tasks, variables, loops, conditions, registered results, handlers, and `become` support;
- ad-hoc module execution;
- local command execution and remote execution through SSH;
- playbook support for `command`, `shell`, `debug`, `copy`, `file`, `template`, `lineinfile`, `user`, `service`, `systemd`, `stat`, `get_url`, and `package`;
- short module names and the `ansible.builtin.*` / `ansible.legacy.*` name prefixes;
- playbook `--limit`, `--check`, and `--forks` options.

Tasks retain playbook order, while hosts within a task run concurrently with a bounded worker count (`--forks`, default 5). Most state-management modules and remote commands assume a POSIX shell, and the `package`, `service`, and `user` modules target Linux-style system tools. Linux is the primary supported and tested platform; Windows support is not claimed.

Module names and common parameters resemble Ansible, but their complete argument schemas, facts, plugin system, transports, strategy engine, Vault support, and all Ansible edge cases are not implemented. Idempotency and check-mode prediction can also differ from Ansible.

Playbook fact gathering follows the ansible-core 2.21 default: facts are gathered
unless a play sets `gather_facts: false`. Fact gathering, `stat`, `command`, and
`shell` use the native Rust worker and do not require Python or curl.
File operations and `get_url` also use native workers. All target modules now use the native worker and argument-array OS tools;
Rustsible does not invoke Python or curl. Package managers such as dnf/yum can
have their own OS-provided Python dependencies.
Registered results and gathered facts last for the complete playbook run;
play and task variables remain scoped to their play or task. Explicit `setup: {}`
merges facts just like automatic gathering, including time collected on the
managed host. With gathering disabled, Rustsible does not invent target facts.
Failed and unreachable hosts remain excluded in subsequent plays, while healthy
hosts continue. `ignore_errors` applies to module failures, not unreachable
hosts. A single recap totals the complete run.

Arguments, conditions, loops, and template files use MiniJinja's Jinja syntax.
A single expression preserves its type (`{{ '0012' }}` stays a string); mixed
text stays text. Only variable definitions resolve references recursively,
with cycle detection. Runtime results and rendered output are never executed
again as templates. Conditions must return booleans; the string `'false'` is
rejected. Use Jinja positional defaults such as `default([])` rather than the
former Tera spelling `default(value=[])`. Relative `copy.src` and `template.src`
paths are anchored to the task's source file, independent of the launch directory.

Inventory host/group variables each use a single typed table. The effective
order is inventory groups → host → facts → play vars → task vars → registered
results → loop variables. Connection variables are resolved from that same
context and override play/task keyword defaults; aliases are normalized within
each layer. Invalid connection types or port values, unresolved references,
and variable cycles fail before connecting or modifying a target. Only sudo is
implemented for become; custom become executables and flags are rejected.
Ad-hoc CLI become overrides retain their documented Rustsible precedence.
The compatibility surface currently includes Linux architecture facts,
`stat.path/checksum_algorithm`, `get_url.url/dest/owner/group/mode/force/checksum`
(inline `<algorithm>:<digest>` checksums),
`systemd.name/enabled/state/daemon_reload`, `copy.force`, and `default(omit)`.
The systemd module rejects non-systemd hosts explicitly.

`copy`, `template`, `lineinfile`, and `get_url` prepare content in a private
same-filesystem directory and commit it with an atomic rename. File operations
use directory descriptors to pin the destination across ancestor renames. The
supervisor owns staging directories and cleans them after module success,
failure, timeout or disconnect; killed module processes cannot leave staging
cleanup to their own destructors. Sensitive content uses `0600` staging files
inside `0700` directories. Controller file content is length-framed and streamed
separately from JSON, including files larger than the 16 MiB command-output cap.
Copy/template inline content also preserves NUL bytes.

Replacement preserves existing mode, uid/gid, POSIX ACLs and extended attributes,
including capabilities. Ownership precedes ACL/xattr/mode restoration, so chown
cannot clear a protection after it has been restored. An explicit mode may change
the ACL mask, but named ACL entries are retained. Verification, write failures,
short transfers, full disks and detected destination changes leave the original
file intact. Destinations for content installation must be regular files or
absent; symbolic links are rejected. The staging parent must be readable/searchable/writable by the execution
identity, owned by root or that identity, and use sticky-bit protection when
group/other-writable.
Metadata-only changes keep the inode and avoid rewriting content. Creating or
repairing file metadata needs no read permission on the file. Recursive directory creation applies requested attributes to each
new directory. Symlinks and removal use native filesystem calls.

`get_url` uses blocking reqwest with Rustls on the target, so target DNS, HTTP(S)
proxy environment and native certificate roots remain authoritative. Certificate
verification stays enabled; `SSL_CERT_FILE`/`SSL_CERT_DIR` can select target CA
roots. Sudo retains its normal environment policy. HTTP(S) redirects are limited
to ten, connections to thirty seconds, and the overall request to the module
budget (`rustsible_command_timeout`). Downloads stream without a response-size
memory limit. With `force: false`, an existing file is retained without network
traffic unless its supplied checksum differs; metadata changes still apply.
Check mode uses HEAD for a required download and never creates destination
staging files. Supported inline checksums are md5, sha1, sha224, sha256, sha384,
and sha512; unknown options and invalid mode/identity/checksum inputs fail before
network traffic or business modifications.

`lineinfile` uses the last regexp/insertion match and handles line endings like
the pinned Ansible 2.21 reference. It preserves no-op bytes, adds a missing final
newline when ensuring that line, and preserves backup ownership/mode/ACL/xattrs
and timestamps. Its current text-input bound is 64 MiB and its regexp language
is Rust regex; Python-only regexp constructs are rejected. Templates render
only on the controller.

User, package and service tasks execute under the worker's selected identity;
they do not add implicit sudo. User passwords/hashes go to `chpasswd -e` over
stdin. Account groups and IDs are resolved before mutation; missing homes for
existing accounts use `mkhomedir_helper`. Both controller frontends and workers
use the same module schemas. OS-tool failures retain rc/stdout/stderr and known
partial changes. A package list uses one install/remove transaction, cache
refresh precedes candidate queries, and check never refreshes cache.

Each task has a temporary subreaper supervisor. Shell/command descendants are
terminated and reaped on completion or cancellation. Successful system-service
and package daemons pass to PID 1 when that supervisor exits, rather than being
adopted by the persistent worker and killed by a subsequent task. Cancelled or
unconfirmed operations terminate their owned execution trees.

The current real OS acceptance covers Debian systemd/APT, Alpine OpenRC/APK,
and Debian SysV, including normal/root/non-root-become identities and local/SSH
transports on x86_64 and aarch64. Additional x86_64 fixtures exercise real Rocky
DNF, CentOS Yum, Arch Pacman and openSUSE Zypper against generated local package
repositories, with networking disabled during execution. Ubuntu Upstart runs as
PID 1 and serves a real HTTP daemon across subsequent tasks. Run these fixtures
with `bash tests/managers/run.sh CONTROLLER WORKERS MANAGER` and
`bash tests/managers/upstart.sh CONTROLLER WORKERS HTTP_FIXTURE`.
Zypper `state=latest` remains explicitly
unsupported. Package specifications currently require concrete names; APT
`name=version` is recognized. Existing-account home repair requires the target
OS's `mkhomedir_helper` tool.

## Build

Install a current stable Rust toolchain, then run:

```bash
git clone https://github.com/LeeXN/rustsible.git
cd rustsible
cargo build --workspace --release --locked
cargo run --locked -p rustsible-protocol --example worker_manifest -- \
  target/release/workers "$(uname -m)=target/release/rustsible-worker"
```

The controller is written to `target/release/rustsible`. The command above
creates its adjacent `workers/manifest.json` and a worker for the build host.
For portable Linux workers, build with the matching `*-unknown-linux-musl`
Rust target and a musl C toolchain. Release packages include both x86_64 and
aarch64 static workers and a SHA-256 manifest. Source archives without Git
metadata require an explicit `RUSTSIBLE_BUILD_ID` when compiling.

The controller selects a Linux worker with `uname`, verifies the local artifact,
streams it into a private target directory, verifies its checksum again, then
checks protocol version, build ID, architecture, execution UID and capabilities.
Use `--worker-dir DIRECTORY` to select a different release worker directory and
inventory `rustsible_remote_tmp: /absolute/executable/directory` to replace the
remote `/tmp` default. The parent must exist; it and its ancestors must be owned
by root or the execution identity, with the sticky bit wherever other users can
write. Validation walks actual parent directories, and cleanup verifies the
original directory inode even after an ancestor rename. `noexec`, missing
or damaged artifacts, unsupported architectures and mismatched builds fail
explicitly. No Python fallback is used.

Workers are subprocesses held for one run and connection/identity combination;
there is no service or listener. Directories and executable workers use `0700`,
request scratch and shell programs use `0600`; credentials and module requests
travel through stdin. The controller keeps stdin open to distinguish a complete
request from cancellation. Closing it cancels synchronous subprocesses, including
`setsid` descendants. Both the worker and bootstrap clean the private directory,
so an abrupt disconnect does not rely solely on the shell's exit trap. Command
processes retain the original working directory and execution identity's umask. Interrupted modifying
requests are reported as uncertain and are never replayed.

To assemble an offline package after building both static workers:

```bash
bash scripts/package.sh target/release/rustsible target/package-linux \
  target/x86_64-unknown-linux-musl/release/rustsible-worker \
  target/aarch64-unknown-linux-musl/release/rustsible-worker
```

Extract the resulting `.tar.gz`, verify `sha256sum -c SHA256SUMS` from its root,
and run `./rustsible`. Packaging requires Cargo; running an extracted release
does not. Workers need SSH for remote access, POSIX shell, `uname`, `id`, `stat`,
`mktemp`, `head`, `wc`, `sha256sum`, `cut`, `chmod`, and `rm` for bootstrap.
Sudo is required only for become.

## Usage

Run a playbook:

```bash
rustsible playbook site.yml -i inventory.ini
```

Limit execution to a host or group expression:

```bash
rustsible playbook site.yml -i inventory.ini --limit webservers
```

Preview a playbook without applying state changes:

```bash
rustsible playbook site.yml -i inventory.ini --check
```

In check mode, commands with no safe prediction (`command` and `shell`) are skipped. State-management modules report their predicted result without applying it. Prediction is best effort and is not a substitute for testing against a disposable target.

Control the maximum number of hosts processed concurrently:

```bash
rustsible playbook site.yml -i inventory.ini --forks 10
```

Run an ad-hoc module:

```bash
rustsible ad-hoc webservers -i inventory.ini -m command -a "uname -a"
rustsible ad-hoc all -i inventory.ini -m file -a "path=/tmp/example state=touch mode=0644"
rustsible ad-hoc all -i inventory.ini -m package -a "name=curl state=present" --become --check
rustsible ad-hoc all -i inventory.ini -m ansible.builtin.setup
rustsible ad-hoc all -i inventory.ini -m stat -a '{"path":"/usr/local/bin/rs-cmdb-client","checksum_algorithm":"sha1"}'
```

Ad-hoc runs inherit `ansible_become` and `ansible_become_user` per host. Use
`--become`, `--become-user`, `--check`, and `--forks` to override execution
controls from the command line. Module arguments accept Ansible-style
`key=value` input or a JSON object. `command` and `shell` additionally accept a
JSON string or `{"cmd":"..."}`. Structured module data such as `ansible_facts`,
`stat`, and `checksum_src` is printed as JSON. As in Ansible, ad-hoc fact
gathering uses the `setup` module rather than the play-only `gather_facts`
keyword.

Playbook and ad-hoc execution share one module registry, argument schema,
capability policy, dispatcher, and common result conversion. Unsupported keys
and wrong types fail before dispatch, including during check mode. The check
policy is read-only for `debug/setup/stat`, predictive for state modules, and
skip for commands without a safe prediction. Diff and async remain unsupported
and are rejected; later implementation stages will enable them explicitly.

Results distinguish `ok`, `failed`, `skipped`, `unreachable`, and `uncertain`
through `outcome` and the `failed/skipped/unreachable/uncertain` flags. `rc`, output, and confirmed
earlier changes survive a failed operation. A lost response after a potentially
modifying execution is `uncertain`; it is not replayed, is not swallowed by
`ignore_errors`, and excludes that host from later work. The same rule stops
remaining loop items. `no_log` still censors user-facing payloads and diagnostics.
See [examples/module_contract.yml](examples/module_contract.yml).

Inspect the parsed inventory without running tasks:

```bash
rustsible inventory-debug -i inventory.ini
```

Use `rustsible <subcommand> --help` for the complete CLI syntax.

## Inventory

A minimal INI inventory looks like this:

```ini
[webservers]
web1 ansible_host=192.0.2.10 ansible_user=deploy
web2 ansible_host=192.0.2.11 ansible_port=2222

[local]
localhost ansible_connection=local

[webservers:vars]
ansible_ssh_private_key_file=/home/deploy/.ssh/id_ed25519
```

YAML inventory is also accepted:

```yaml
all:
  children:
    webservers:
      hosts:
        web1:
          ansible_host: 192.0.2.10
          ansible_user: deploy
```

Inventory variables may contain credentials. Keep inventory files out of source control, restrict their file permissions, and prefer SSH keys over passwords.

## Playbook example

```yaml
---
- name: Configure web nodes
  hosts: webservers
  become: true
  vars:
    config_path: /etc/example.conf

  tasks:
    - name: Install the package
      package:
        name: nginx
        state: present

    - name: Render configuration
      template:
        src: templates/example.conf.j2
        dest: "{{ config_path }}"
        mode: "0644"
      notify: restart nginx

  handlers:
    - name: restart nginx
      service:
        name: nginx
        state: restarted
```

Examples under [`examples/`](examples/) include runnable subset examples plus explicitly documented migration fixtures that exercise unsupported Ansible syntax. They are examples rather than a compatibility guarantee.

## SSH security defaults

SSH host-key verification is enabled by default. Rustsible reads the system OpenSSH known-hosts file and `$HOME/.ssh/known_hosts`; an unknown or mismatched key causes the connection to fail.

For an isolated known-hosts file, set one of these inventory variables:

```ini
host1 rustsible_known_hosts_file=/path/to/known_hosts
# ansible_ssh_known_hosts_file is also recognized
```

Host-key verification can be disabled only through an explicit inventory setting:

```ini
host1 rustsible_host_key_checking=false
```

`ansible_host_key_checking` and `ansible_ssh_host_key_checking` are also recognized. Disabling verification makes SSH connections vulnerable to interception and should only be used in controlled, temporary environments.

The default SSH connection timeout is 10 seconds and the default remote-command timeout is 300 seconds. They can be configured with `ansible_ssh_timeout` / `ansible_timeout` and `rustsible_command_timeout` / `ansible_command_timeout`, respectively.

## Supported modules

The following built-in modules are registered for playbooks:

| Module | Purpose |
| --- | --- |
| `command` | Run a command |
| `shell` | Run a command through a shell |
| `debug` | Display a message or variable |
| `copy` | Copy a controller file or literal content |
| `file` | Manage files, directories, links, and permissions |
| `template` | Render a MiniJinja/Jinja template |
| `lineinfile` | Add, replace, or remove matching lines |
| `user` | Manage local user accounts |
| `service` | Manage service state and enablement |
| `systemd` / `systemd_service` | Manage systemd units and daemon reloads |
| `setup` | Gather the supported Linux facts |
| `stat` | Return path existence and checksums |
| `get_url` | Download HTTP/HTTPS resources on the managed host |
| `package` | Manage packages through detected system tools |

Only the subset exercised by this repository's tests should be considered supported. Consult the module source and examples before relying on Ansible-specific parameters.

## Development and testing

See [DEVELOPMENT_STATUS.md](DEVELOPMENT_STATUS.md) for the staged plan's
implemented items, validation evidence, and outstanding work.

The CI-equivalent local checks are:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo build --workspace --locked
cargo test --workspace --all-targets --all-features --locked
cargo llvm-cov --workspace --all-targets --all-features --locked --fail-under-lines 80
cargo build --workspace --release --all-features --locked
```

To run a focused test, pass its name to Cargo, for example:

```bash
cargo test modules::file
```

For a staged hands-on acceptance run covering local execution, SSH, check mode,
idempotency, privilege escalation, failure handling, concurrency, and log
redaction, follow [`MANUAL_TEST_PLAN.md`](MANUAL_TEST_PLAN.md).

`bash tests/ssh/run.sh target/debug/rustsible` runs an isolated Docker SSH target
with strict host-key verification. It checks deploy/root/app execution identities,
check-mode business-state equality, idempotency, cross-play facts/registers,
file bytes and metadata, and temporary-file cleanup. The full fixture installs
neither Python nor curl.
`bash tests/ssh/run.sh CONTROLLER WORKER_DIRECTORY native glibc HTTP_FIXTURE`
and the same command with `musl` instead of `glibc` use targets with neither Python nor curl. They
verify native setup/stat/command/shell, files/downloads, password sudo,
root/non-root execution, 20 MiB binary transfers, HTTPS/private CA/proxy,
check invariance, noexec failures, real disk-full preservation and cleanup, including input-byte/inode exhaustion. The musl
case also runs the static controller inside the target. CI runs these scenarios
on native x86_64 and aarch64 runners and assembles offline bundles with both
workers.
CI runs the quality gates and real SSH fixture on native x86_64 and aarch64
Linux runners. Results come from the individual CI jobs; configuring a runner
is not itself evidence that an architecture passed.

`bash tests/reference/run.sh target/debug/rustsible` compares template and
condition results with **ansible-core 2.21.0**. It uses `uv` to isolate the
Python reference tool; reference dependencies are not needed by normal Cargo
tests. The comparison checks target file bytes, including literal runtime
output, defaults/omit, loop values, string types, and rejected string conditions.

Dependency advisories are checked in CI against `Cargo.lock`. CI also enforces
an 80% line-coverage floor. This is a regression baseline, not a claim that every remote SSH path is exercised end to
end.

## License

Rustsible is available under the [MIT License](LICENSE).

## Acknowledgements

Rustsible is inspired by [Ansible](https://github.com/ansible/ansible) and uses names familiar to Ansible users. It is an independent implementation and is not affiliated with or endorsed by Red Hat or the Ansible project.
