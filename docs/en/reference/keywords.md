[English / 简体中文](../../zh-CN/reference/keywords.md) · [Index / 目录](../index.md)

# Playbook keywords

<a id="supported_play_keywords"></a>

## Play keywords

`strategy`, `serial`, `throttle`, `run_once`, `any_errors_fatal`, `max_fail_percentage`, `force_handlers`, `ignore_errors`, `ignore_unreachable`, `no_log`, `pre_tasks`, `post_tasks`, `roles`, `vars_files`, `name`, `environment`, `hosts`, `tasks`, `handlers`, `vars`, `become`, `become_user`, `connection`, `fail_fast`, `gather_facts`, `gather_subset`, `gather_timeout`, `collections`, `tags`

<a id="supported_task_keywords"></a>

## Task keywords

`async`, `poll`, `delegate_to`, `delegate_facts`, `throttle`, `run_once`, `any_errors_fatal`, `ignore_unreachable`, `name`, `args`, `environment`, `timeout`, `delay`, `retries`, `until`, `failed_when`, `changed_when`, `become`, `become_user`, `check_mode`, `connection`, `register`, `when`, `notify`, `ignore_errors`, `no_log`, `vars`, `with_items`, `loop`, `loop_control`, `tags`

<a id="blocks"></a>

## Structural forms and handlers

block/rescue/always and import/include directives are structural forms, not modules. Handlers additionally accept listen and notify topics; force_handlers and phase boundaries control flushing. loop_control accepts loop_var/index_var only. roles support public, allow_duplicates, selected *_from files and dependencies.

<a id="defaults"></a>

## Important defaults and restrictions

gather_facts:true; strategy:linear; retries:3 additional attempts with until; delay:5 seconds; async:0; poll:15 seconds. timeout is a task attempt budget, not the URI/wait timeout parameter. free rejects run_once/fatal/percentage controls. Unsupported keywords, including vars_prompt/module_defaults/task collections, fail; see compatibility.
