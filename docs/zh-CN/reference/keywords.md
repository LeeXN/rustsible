[English / 简体中文](../../en/reference/keywords.md) · [Index / 目录](../index.md)

# Playbook 关键词

<a id="supported_play_keywords"></a>

## Play 关键词

`strategy`, `serial`, `throttle`, `run_once`, `any_errors_fatal`, `max_fail_percentage`, `force_handlers`, `ignore_errors`, `ignore_unreachable`, `no_log`, `pre_tasks`, `post_tasks`, `roles`, `vars_files`, `name`, `environment`, `hosts`, `tasks`, `handlers`, `vars`, `become`, `become_user`, `connection`, `fail_fast`, `gather_facts`, `gather_subset`, `gather_timeout`, `collections`, `tags`

<a id="supported_task_keywords"></a>

## Task 关键词

`async`, `poll`, `delegate_to`, `delegate_facts`, `throttle`, `run_once`, `any_errors_fatal`, `ignore_unreachable`, `name`, `args`, `environment`, `timeout`, `delay`, `retries`, `until`, `failed_when`, `changed_when`, `become`, `become_user`, `check_mode`, `connection`, `register`, `when`, `notify`, `ignore_errors`, `no_log`, `vars`, `with_items`, `loop`, `loop_control`, `tags`

<a id="blocks"></a>

## 结构与 handler

block/rescue/always 与 import/include 是结构形式，不是模块。handler 另支持 listen、notify 主题，force_handlers 和阶段边界控制刷新。loop_control 仅接受 loop_var/index_var；roles 支持 public、allow_duplicates、指定 *_from 文件与依赖。

<a id="defaults"></a>

## 重要默认值与限制

gather_facts:true；strategy:linear；until 默认额外 retries:3；delay:5秒；async:0；poll:15秒。任务 timeout 是尝试预算，与 URI/wait 模块参数不同。free 拒绝 run_once/fatal/百分比组合；vars_prompt/module_defaults/任务级 collections 等不支持关键词会报错，详见兼容性。
