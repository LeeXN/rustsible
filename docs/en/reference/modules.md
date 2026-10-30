[English / 简体中文](../../zh-CN/reference/modules.md) · [Index / 目录](../index.md)

# Builtin module catalog

The effective validator takes precedence over a shared parameter superset. Check may predict, query or skip; see each page.

| Module | Public parameters | Check | Diff | Async |
|---|---|---|---|---|
| [command](modules/command.md) | `cmd`, `argv`, `chdir`, `creates`, `removes`, `stdin`, `stdin_add_newline`, `strip_empty_ends`, `expand_argument_vars` | guarded | false | true |
| [shell](modules/shell.md) | `cmd`, `chdir`, `creates`, `removes`, `stdin`, `stdin_add_newline`, `strip_empty_ends`, `expand_argument_vars` | guarded | false | true |
| [debug](modules/debug.md) | `msg`, `var` | readonly | false | false |
| [meta](modules/meta.md) | `action` | readonly | false | false |
| [set_fact](modules/set_fact.md) | `cacheable`, `<variable>` | readonly | false | false |
| [include_vars](modules/include_vars.md) | `file`, `dir`, `name` | readonly | false | false |
| [copy](modules/copy.md) | `src`, `content`, `dest`, `mode`, `owner`, `group`, `force` | prediction | true | false |
| [file](modules/file.md) | `path`, `dest`, `state`, `src`, `mode`, `owner`, `group` | prediction | true | true |
| [template](modules/template.md) | `src`, `content`, `dest`, `mode`, `owner`, `group`, `vars` | prediction | true | false |
| [lineinfile](modules/lineinfile.md) | `path`, `line`, `regexp`, `state`, `backup`, `create`, `insertafter`, `insertbefore`, `owner`, `group`, `mode` | prediction | true | true |
| [user](modules/user.md) | `name`, `state`, `uid`, `gid`, `group`, `groups`, `append`, `home`, `shell`, `comment`, `password`, `create_home`, `system`, `remove` | prediction | false | true |
| [service](modules/service.md) | `name`, `state`, `enabled` | prediction | false | true |
| [systemd](modules/systemd.md) | `name`, `state`, `enabled`, `daemon_reload` | prediction | false | true |
| [setup](modules/setup.md) | `gather_subset`, `filter`, `gather_timeout` | readonly | false | false |
| [stat](modules/stat.md) | `path`, `checksum_algorithm` | readonly | false | false |
| [get_url](modules/get_url.md) | `url`, `dest`, `owner`, `group`, `mode`, `force`, `checksum` | prediction | true | true |
| [package](modules/package.md) | `name`, `state`, `update_cache` | prediction | false | true |
| [async_status](modules/async_status.md) | `jid`, `mode` | prediction | false | false |
| [assert](modules/assert.md) | `that`, `fail_msg`, `msg`, `success_msg`, `quiet` | readonly | false | false |
| [fail](modules/fail.md) | `msg` | readonly | false | false |
| [group](modules/group.md) | `name`, `gid`, `state`, `system` | prediction | false | false |
| [wait_for](modules/wait_for.md) | `host`, `port`, `path`, `state`, `search_regex`, `timeout`, `connect_timeout`, `delay`, `sleep` | skip | false | false |
| [uri](modules/uri.md) | `url`, `method`, `headers`, `body`, `body_format`, `status_code`, `timeout`, `return_content`, `validate_certs`, `use_proxy`, `ca_path`, `follow_redirects`, `url_username`, `url_password`, `user`, `password`, `force_basic_auth` | skip | false | false |
| [git](modules/git.md) | `repo`, `dest`, `version`, `update` | prediction | true | false |
| [unarchive](modules/unarchive.md) | `src`, `dest`, `remote_src`, `owner`, `group`, `mode`, `creates` | prediction | true | false |
