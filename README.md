# Rustsible

[![CI](https://github.com/LeeXN/rustsible/actions/workflows/rust.yml/badge.svg)](https://github.com/LeeXN/rustsible/actions/workflows/rust.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

Rustsible is an experimental, Ansible-inspired configuration and command runner written in Rust.

> Rustsible is not a drop-in replacement for Ansible. It implements a useful subset of Ansible's inventory, playbook, templating, and module behavior, but compatibility is incomplete. Review a playbook with `--check` and test it in a disposable environment before using it on important systems.

## Current scope

Rustsible currently provides:

- INI and YAML inventory parsing, group variables, child groups, and host-pattern filtering;
- playbook imports, task includes, roles, blocks, variables, loops, conditions, registered results, handlers, and `become` support;
- ad-hoc module execution;
- local command execution and remote execution through SSH;
- playbook support for `command`, `shell`, `debug`, `copy`, `file`, `template`, `lineinfile`, `user`, `service`, `systemd`, `stat`, `get_url`, and `package`;
- short module names and the `ansible.builtin.*` / `ansible.legacy.*` name prefixes;
- playbook `--limit`, `--check`, and `--forks` options.

The default `linear` strategy retains task barriers; `free` lets hosts advance independently. Both bound active host controllers with `--forks` (default 5). Most state-management modules and remote commands assume a POSIX shell, and the `package`, `service`, and `user` modules target Linux-style system tools. Linux is the primary supported and tested platform; Windows support is not claimed.

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

Inventory host/group variables each use a single typed table. Variable order
is inventory groups → host → facts → play vars → vars_files → task vars →
include_vars → set_fact/register → extra vars. Transient loop bindings and the
current attempt's register are available within that execution; subsequent
tasks again see extra vars at their normal highest precedence. Connection
variables use that same context and override play/task keyword defaults.
Configuration is a shared, literal baseline below keywords and variables.
Aliases normalize within each layer. Invalid connection types/ports,
unresolved references, and variable cycles fail before target operations.
Only sudo is implemented for become; custom executables/flags are rejected.
Ad-hoc CLI become overrides retain their documented Rustsible precedence.

| Variable source | Supported form |
| --- | --- |
| `group_vars` / `host_vars` | Inventory directory, then playbook directory; extensionless/YAML/JSON files or sorted directories; host values outrank groups |
| `vars_files` | Source-relative paths, including ordered fallback lists; fact-dependent paths resolve after gathering |
| `include_vars` | Exactly one of `file`/`dir`, optional namespace `name`; directories recursively load supported files in lexical order |
| `set_fact` | Typed in-memory values, including loop accumulation, retained across plays; persistent `cacheable: true` is rejected |
| `-e` / `--extra-vars` | Repeated `name=value` strings, YAML/JSON objects, or `@file`; structured inputs retain their types |

`hostvars`, `groups`, `group_names`, `inventory_hostname`, `inventory_file`,
`inventory_dir`, `playbook_dir`, and the selected `ansible_config_file` are
available to templates. Hostvars includes inventory and persistent host values,
including hosts outside the current play. Its view advances at task boundaries;
qualified hostvars inside a loop therefore observes the preceding task, as in
Ansible 2.21. Ad-hoc arguments now share the playbook renderer. To embed literal
template delimiters, return them from a quoted expression or variable; rendered
output is never evaluated again. See [the variable example](examples/variables/playbook.yml).

`--config FILE` selects a required configuration file. Otherwise Rustsible
uses `ANSIBLE_CONFIG`, project `ansible.cfg`, `~/.ansible.cfg`, then
`/etc/ansible/ansible.cfg`. Explicit inventory/forks/worker-dir options override
configuration; supported environment options override file values. Paths in a
configuration file resolve relative to that file. Unsupported keys fail with
their file/section/key instead of silently changing execution expectations.

| Section | Implemented keys |
| --- | --- |
| `[defaults]` | `inventory` (one file), `forks`, `verbosity`, `remote_user`, `private_key_file`, `timeout`, `host_key_checking`, `roles_path`, Rustsible's `worker_dir` |
| `[privilege_escalation]` | `become`, `become_user`, `become_method` (`sudo`) |

The corresponding `ANSIBLE_INVENTORY`, `ANSIBLE_FORKS`, `ANSIBLE_VERBOSITY`,
`ANSIBLE_REMOTE_USER`, `ANSIBLE_PRIVATE_KEY_FILE`, `ANSIBLE_TIMEOUT`,
`ANSIBLE_HOST_KEY_CHECKING`, `ANSIBLE_BECOME`, `ANSIBLE_BECOME_USER`, and
`ANSIBLE_BECOME_METHOD`, and `ANSIBLE_ROLES_PATH` environment settings are recognized.

Facts run on the target. Setup supports `gather_subset`, `filter` (a string or
list of shell patterns), and `gather_timeout` (1–604800 seconds, default 10).
Plays also accept `gather_subset`/`gather_timeout`. Setup results use prefixed
keys such as `ansible_architecture`; the host's `ansible_facts.architecture`
namespace removes that prefix. `set_fact` and `include_vars` retain their own
host-variable names.

| Native subset | Returned fields (with `ansible_` in setup results) |
| --- | --- |
| `platform` | architecture, system, kernel, kernel_version, machine, hostname, nodename, fqdn, domain |
| `distribution` | distribution, distribution_version, distribution_major_version, distribution_release, os_family |
| `date_time` | date/time components, weekday, weekday_number, epoch, iso8601 |
| `network` | interfaces, all_ipv4_addresses, all_ipv6_addresses, default_ipv4 (main routing table) |
| `min` / `all` | platform/distribution/date_time; `all` additionally includes network |

Individual documented architecture/system/kernel/machine, distribution-version,
OS-family, and network selectors are accepted. `!all` keeps minimum facts;
`!all,!min` collects none unless explicit subsets are added. Unsupported subsets
are rejected; filter only returns matching top-level fact names. Python facts,
executable local facts and the rest of Ansible's collectors are not declared.
The compatibility surface currently includes Linux architecture facts,
`stat.path/checksum_algorithm`, `get_url.url/dest/owner/group/mode/force/checksum`
(inline `<algorithm>:<digest>` checksums),
`systemd.name/enabled/state/daemon_reload`, `copy.force`, and `default(omit)`.
The systemd module rejects non-systemd hosts explicitly.

Tasks support `changed_when`, `failed_when`, `until`, `retries`, `delay`,
`timeout`, and play/task `environment`. Conditions use strict booleans; lists
are combined with AND. Registers update after every attempt, including each
loop item. `retries` counts additional attempts (default 3 when `until` is
present); `delay` defaults to 5 seconds. Exhausted `attempts` follows the pinned
Ansible 2.21 retry-count convention. A task timeout limits each target execution
and terminates its descendants; uncertain or unreachable results stop retries.
Task environment values override play values and remain private to the request.

`command` accepts exactly one of `cmd` or `argv`, plus `chdir`, `creates`,
`removes`, `stdin`, `stdin_add_newline`, `strip_empty_ends`, and
`expand_argument_vars`. `shell` accepts the same parameters except `argv`.
Arguments may also use the task's `args` mapping. File guards support globs,
relative to `chdir`, with `creates` checked first. In check mode they predict
changes without running the command; commands without a guard are skipped.
`argv` preserves argument boundaries, and target environment expansion does
not evaluate shell syntax. Unknown variables remain literal; set
`expand_argument_vars: false` to disable expansion. Stdin gains a final newline
and stdout/stderr lose trailing CR/LF by default; the corresponding boolean
parameters can disable either behavior.

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

Playbook and ad-hoc executions share a bounded pool of host threads. A free
thread immediately takes the next host, while playbook tasks retain their
default ordering barrier. A host reuses its authenticated worker connection
until effective connection settings change or `meta: reset_connection` runs.
Changing settings closes the previous lease before creating another. Lost
responses are reported without automatically replaying a modifying request.

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

## Task reuse and failure recovery

`import_playbook`, `import_tasks`, and `import_role` expand during loading, so
invalid static content fails before target operations. `include_tasks` and
`include_role` expand only after their conditions select them; loops retain a
separate variable context for each item. Static filenames can use play/role
definitions and extra variables; use dynamic includes for host/runtime choices.
Each task keeps its actual source file for nested imports, variable files,
`copy` sources, and templates, including when started from another directory.
Imported conditions are checked on each child task; dynamic include conditions
select the include itself. `apply` supplies settings inherited by dynamic children.

Roles support `tasks`, `defaults`, `vars`, `handlers`, and `meta/dependencies`,
inline role parameters, `tasks_from`/`defaults_from`/`vars_from`/`handlers_from`,
`public`, and `allow_duplicates`. Search starts in the playbook's `roles/`, then
configured `roles_path` (colon separated) or the standard user/system role
directories, followed by the parent role's directory and playbook/current
directories. Dependencies run first; ancestry cycles fail with their chain.
Role defaults are below inventory values; role variables, task variables,
persistent host results, role/include parameters, then extra variables have
increasing priority. Play roles and static imports expose their variables to
the play by default. Dynamic roles are private unless `public: true`; public
values become available to subsequent tasks. Nonempty plugin directories and
role `argument_specs` are rejected during role loading.

`pre_tasks`, role/main tasks, and `post_tasks` each finish with a handler flush.
Handlers run in definition order, accept `listen`, deduplicate notifications,
and retain normal register/fact/condition behavior. The last definition wins
for a repeated handler name. `force_handlers: true` runs pending handlers on
hosts with module failures, provided their connection result is still known.

`block`, `rescue`, and `always` inherit task settings and variables. A module
failure enters rescue with `ansible_failed_task` and `ansible_failed_result`;
a successful rescue restores the host and is counted as rescued. Always runs
after normal or rescued module execution, including a failed rescue. An
unreachable host or an uncertain modification result enters neither rescue nor
always. `ignore_errors` applies to module failures; `ignore_unreachable` is a
separate explicit setting. Global check mode and `no_log` also cover these paths.

Playbooks support `meta: flush_handlers`, `clear_host_errors`, `reset_connection`,
`end_host`, and `end_play`. Clear errors allows excluded hosts into later plays;
it preserves the failure recap and exit status. End host/play stops that play
without excluding the host from later plays. Ad-hoc meta supports only reset.
Blocks cannot loop or register as a unit; static imports cannot loop. Tags are
still outside the implemented scope.
See [the reusable deployment example](examples/reuse/main.yml) and the pinned
reference scenarios under [tests/reference/reuse](tests/reference/reuse).

## Rolling deployment and delegation

| Control | Scope and behavior |
| --- | --- |
| `serial` | Play count, integer string, percentage from 0–100%, or list; list entries advance by batch and the last entry repeats. Percentages use the original selected host count, including previously failed hosts. Empty batches are skipped. Nonpositive counts mean the remainder; percentage batches contain at least one host. Templates can use play/role definitions, static vars_files and extra variables. |
| `throttle` | Play/block/task nonnegative integer; caps a task below forks/batch limits, including across free host controllers. Zero removes that throttle limit. |
| `run_once` | Play/block/task boolean; evaluates on the first eligible host of each serial batch. Register/fact data is shared with that batch. Only the executing host receives its handler notifications, dynamic children and execution counts. Implicit gathering still runs on each host. |
| `max_fail_percentage` | Play integer from 0–100; the percentage must be exceeded, rather than equaled. Uses the original batch size and stops subsequent deployment work after unhandled failures. |
| `any_errors_fatal` | Play/block/task boolean; propagates a failure through the current batch and stops unhandled deployment work. Rescue/always still apply. |
| `strategy` | `linear` by default, or `free`; built-in FQCN spellings are accepted. Free rejects run_once, any_errors_fatal and max_fail_percentage combinations before executing static tasks or selected dynamic children. |

`ansible_play_hosts_all` and `ansible_play_batch` expose the selected hosts and
the current eligible batch. Pre/main/post tasks and handlers complete within a
serial batch before the next batch starts. Free preserves each host's sequence
and failure handling while sharing published host values and public role variables.
`fail_fast` remains a Rustsible extension that stops the playbook after a failed play.

`delegate_to` accepts a host name or template, including inside loops.
Arguments retain the logical host's variables and `inventory_hostname`;
connection and become options resolve against the delegated host. Register and
failure state belong to the logical host. Facts also belong to it by default;
`delegate_facts: true` assigns them to the delegated host, including per-item
loop targets. An implicit localhost uses local execution. Logical hosts retain
independent workers when delegating concurrently to one target; use throttle
when shared files require serialization.

The pinned Ansible 2.21 behavior of fatal recovery has an additional consequence:
rescue restores execution within the play, while peer hosts stopped by fatal
propagation remain excluded from later plays until clear_host_errors. The run
retains a failure exit status. Ordinary rescued module failures keep their
existing successful recovery behavior.

See [the rolling example](examples/rolling/playbook.yml). The SSH strategy
fixture uses three distinct servers and checks physical identities, batch
barriers, cross-host facts, parallel delegation, idempotency, check and cleanup.

## Target readiness and HTTP

`wait_for` supports a TCP host/port, file presence/absence, or a multiline
`search_regex` over a regular UTF-8 file. Port/path are mutually exclusive;
regex matching requires a path and reads at most 8 MiB. States are
started/present and stopped/absent. Timeout defaults to 300 seconds,
connect_timeout to 5, delay to zero, and sleep to one; timers use nonnegative
whole seconds, with a positive connect timeout. With no condition it sleeps.
For conditions, delay counts toward timeout and timeout=0 expires without a
probe. Absent/stopped file waits test disappearance, regardless of search_regex.
It performs read-only queries and is skipped in global check mode.

`uri` executes on the target using the get_url client/trust/proxy path.
It supports HTTP method, string headers, raw/JSON/form-urlencoded bodies,
integer/list `status_code` (default 200), positive timeout (default 30), and
`return_content`. JSON content types are parsed into `json` independently of
return_content. Requested content must be UTF-8; request/response bodies are
limited to 8 MiB. Unexpected statuses retain status and returned error content.
URI reports changed=false; use changed_when when the API's action requires it.

TLS verification defaults on. `ca_path` adds a target PEM CA bundle (up to
4 MiB); native roots and target proxy variables remain available. use_proxy
can disable proxies. follow_redirects is safe by default (GET/HEAD), none, or
all, with a ten-redirect limit. Basic auth accepts url_username/url_password
(user/password aliases); force_basic_auth controls preemptive authentication.
Credential-bearing responses retain raw register data and a censored display
policy. URLs in errors omit credential-bearing URL data. URI check skips the
request. Client deadlines and task timeout also bound waits and HTTP operations.

## Git and archives

`git` requires target Git and accepts repo, absolute dest, version (HEAD by
default, branch, tag, full ref or full commit), and update (default true).
New clones install from a private sibling directory. Updates reject local
changes; update=false retains an existing checkout. Check uses ls-remote and
local queries without clone/fetch/checkout. Results include before/after commit
IDs. Ambiguous branch/tag names require a full ref; missing tools fail directly.
HTTP URL credentials and external remote helpers are rejected; use target SSH
keys/known_hosts or credential helpers. Submodules and force/reset are outside
this subset. Diff currently has no CLI entry; the common capability validator
rejects a diff request until the CLI/diff delivery.

`unarchive` supports tar, tar.gz and ZIP (stored/deflate), controller src by
default or absolute target src with remote_src=true, absolute existing dest,
owner/group/mode, and creates below dest. Controller paths use the task/role
files source context. It fully decodes into private scratch before touching
business files. Paths escaping dest, archive symlinks/hardlinks/devices,
ambiguous duplicate files, existing link walks and file/directory conflicts
fail before installation. Limits are 100,000 entries and 8 GiB compressed or
expanded content; matching files are unchanged. Existing file protection
metadata is retained unless specified; new entries use archive permission bits
without set-id, limited by the original umask. Existing directory modes remain
unless mode was specified. Archive ownership/timestamps are not imported.
Each replacement is atomic; a later IO failure can leave earlier complete
entries installed and reports those changes. Private staging is cleaned.
Global check decodes and compares without changing the destination. Source
streaming and archive operations do not support async.

## Assertions and groups

`assert` and `fail` run on the controller and do not require a target connection.
Assert accepts `that` as a raw boolean expression, boolean, or a list of those;
the same strict boolean evaluator handles when/until/result conditions. Its
string messages are `fail_msg` (`msg` alias) and `success_msg`; `quiet` suppresses
the successful message while register retains it. String/list message mixtures
and templated expression containers are outside this subset. Runtime output is
data and is never evaluated again as an assertion. Fail accepts a string `msg`.
Both honor no_log and work in check and block rescue paths. See
[the assertion example](examples/assertions.yml).

Native `group` accepts name, optional integer gid, present/absent state and the
creation-only system boolean. Account queries and groupadd/groupmod/groupdel
use the existing account/tool path; standard administrative directories are
also searched when an ordinary SSH PATH omits them. GID conflicts and parameter
types fail before mutation. Check queries and predicts; it does not modify the
group database. Missing tools and primary-group deletion failures are explicit.

## Native background tasks

Tasks accept `async: SECONDS` and `poll: SECONDS`; ad-hoc accepts `-B/--background`
and `-P/--poll`. A zero async duration runs synchronously. Poll defaults to 15;
poll zero returns `ansible_job_id`, `started` and `finished` immediately. Positive
poll waits for a terminal result and cleans its state, retaining output/rc in
the registered result. Async and poll values are nonnegative whole seconds,
bounded to seven days. An explicit task timeout can further limit execution.

Command/shell, target file/lineinfile/get_url, user, service/systemd and package
support async. Copy/template transfers and controller actions do not. Global
check skips a supported async task without starting a process or creating job
state; guard dependent queries with `when: background is not skipped`.

`async_status` accepts `jid` and optional `mode: status` (default) or `cleanup`.
Status returns boolean `started`/`finished`; `result is finished` works with
until/retries/delay. Poll-zero jobs retain their results until explicit cleanup.
Cleanup is idempotent for absent state and rejects running/unconfirmed jobs.
Its check mode predicts deletion. Invalid IDs, corrupt results, unsafe modes,
symlinks and another execution identity's cache are rejected.

State defaults to the execution identity's `~/.rustsible_async`; inventory
`ansible_async_dir` can select an absolute path or target-relative `~/` path.
Directories are `0700`, results are atomically replaced `0600` files, and the
native program retained for a running job is `0700`. The configured directory
must permit execution. Startup checks `noexec` explicitly. A detached native
supervisor enforces the deadline and reaps even setsid descendants; it installs
no service. SSH/controller exit after startup leaves the job running. Credentials
and requests use stdin. Later status queries retain the original job's no_log
display policy, while registered data remains usable in memory.

Protocol v4 adds native job and display metadata; mismatched workers are rejected.
See [the async example](examples/async/playbook.yml) and the
[fixed Ansible 2.21 comparison](tests/reference/async.yml).

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
skip for commands without a safe prediction. Async support follows the module
capability declarations above. Diff remains unsupported and is rejected.

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
| `set_fact` | Retain typed host values during a playbook run |
| `include_vars` | Load controller variable files or directories |
| `async_status` | Query or clean a native background job for the execution identity |
| `assert` | Evaluate raw boolean conditions on the controller, with success/failure messages and quiet display |
| `fail` | Fail deliberately with a message on the controller |
| `group` | Manage Linux group names/GIDs/present/absent/system through account tools |
| `wait_for` | Wait for target TCP/file/regular-text conditions |
| `uri` | Call target HTTP APIs with bounded body/status/TLS/proxy handling |
| `git` | Deploy pinned target Git checkouts without discarding local changes |
| `unarchive` | Validate tar/gzip/ZIP before atomic per-file installation |
| `meta` | Flush handlers, clear host errors, reset connections, or end host/play execution |
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
It also compares imported tasks, role dependencies/parameters/templates,
handler phases and listen topics, and block rescue/always file results.
Rolling, run_once notification/count behavior, failure thresholds, delegation,
and free progression/public role scope have separate reference scenarios.
`bash tests/ssh/run.sh CONTROLLER WORKER_DIRECTORY reuse [glibc|musl]`
checks that deployment through real SSH with three execution identities,
including check-mode business-state equality and worker cleanup.
`bash tests/ssh/run.sh CONTROLLER WORKER_DIRECTORY strategy [glibc|musl]`
runs the three-server deployment/delegation scenarios.
`bash tests/ssh/run.sh CONTROLLER WORKER_DIRECTORY async [glibc|musl]`
checks disconnect survival, state permissions/identity isolation, background
native files, poll/cleanup, no_log, detached descendants and noexec.
`bash tests/ssh/run.sh CONTROLLER WORKERS network [glibc|musl] HTTP_FIXTURE`
checks three identities, HTTP/TLS/private CA/proxy, authentication, until
readiness, global check and cleanup. Reference comparison also needs the
http_fixture example executable, passed as its optional second argument.

Dependency advisories are checked in CI against `Cargo.lock`. CI also enforces
an 80% line-coverage floor. This is a regression baseline, not a claim that every remote SSH path is exercised end to
end.

## License

Rustsible is available under the [MIT License](LICENSE).

## Acknowledgements

Rustsible is inspired by [Ansible](https://github.com/ansible/ansible) and uses names familiar to Ansible users. It is an independent implementation and is not affiliated with or endorsed by Red Hat or the Ansible project.
