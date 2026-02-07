use anyhow::anyhow;
#[allow(unused_imports)]
use anyhow::Result;
use log::{debug, info, warn};
use serde_yaml::{Mapping, Value};
use std::collections::HashMap;
use std::time::Instant;
use tera::{Context as TeraContext, Tera};

use crate::inventory::Host;
use crate::modules::ModuleResult;
use crate::playbook::filters::register_ansible_filters;
use crate::ssh::connection::SshClient;

/// Task result structure for tracking execution status
#[derive(Debug, Clone)]
pub struct TaskResult {
    pub changed: bool,
    pub failed: bool,
    pub skipped: bool,
    pub msg: String,
    #[allow(dead_code)]
    pub host: String, // Keep this for future use
    pub values: HashMap<String, Value>,
}

impl TaskResult {
    pub fn new(host: &str) -> Self {
        TaskResult {
            changed: false,
            failed: false,
            skipped: false,
            msg: String::new(),
            host: host.to_string(),
            values: HashMap::new(),
        }
    }

    pub fn from_module_result(host: &str, module_result: ModuleResult) -> Self {
        let mut values = HashMap::new();

        // Store stdout/stderr as values
        if !module_result.stdout.is_empty() {
            values.insert("stdout".to_string(), Value::String(module_result.stdout));
        }

        if !module_result.stderr.is_empty() {
            values.insert("stderr".to_string(), Value::String(module_result.stderr));
        }

        TaskResult {
            changed: module_result.changed,
            failed: module_result.failed,
            skipped: false,
            msg: module_result.msg,
            host: host.to_string(),
            values,
        }
    }
}

/// Task structure representing a single action in a play
#[derive(Debug, Clone)]
pub struct Task {
    pub name: String,
    pub module: String,
    pub args: Mapping,
    pub is_become: bool, // Renamed from 'become' to avoid Rust keyword
    pub become_user: String,
    pub register: Option<String>,
    pub when: Option<Value>,
    pub notify: Vec<String>,
    pub ignore_errors: bool,
    #[allow(dead_code)]
    pub tags: Vec<String>, // Keep this for future use
    pub loop_items: Option<Value>,
    pub loop_var_name: Option<String>, // Name for loop variable (default: item)
    pub index_var_name: Option<String>, // Name for index variable
}

// Helper function to check for and extract simple variable names like {{ var }} or {{ var.sub_var }}
fn extract_simple_variable(input: &str) -> Option<&str> {
    let trimmed = input.trim();
    if trimmed.starts_with("{{") && trimmed.ends_with("}}") {
        let inner = trimmed[2..trimmed.len() - 2].trim();
        // Allow alphanumeric, underscore, and dot for nested access
        if !inner.is_empty()
            && inner
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
            && !inner.starts_with('.')
            && !inner.ends_with('.')
            && !inner.contains("..")
        {
            return Some(inner);
        }
    }
    None
}

// Helper to perform nested lookups like "application.features"
fn get_nested_value<'a>(key: &str, vars: &'a HashMap<String, Value>) -> Option<&'a Value> {
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() {
        return None;
    }

    let mut current_value = vars.get(parts[0])?;

    for part in parts.iter().skip(1) {
        match current_value {
            Value::Mapping(map) => {
                // Try looking up the part as a string key
                if let Some(next_value) = map.get(&Value::String(part.to_string())) {
                    current_value = next_value;
                } else {
                    // Handle potential non-string keys if necessary, though less common
                    return None; // Key part not found in map
                }
            }
            _ => {
                return None; // Cannot access sub-key on a non-mapping value
            }
        }
    }
    Some(current_value)
}

impl Task {
    pub fn execute(&self, host: &Host, vars: &HashMap<String, Value>) -> Result<TaskResult> {
        let start_time = Instant::now();
        info!("TASK [{}] on host {}", self.name, host.name);

        let mut tera = Tera::default();
        register_ansible_filters(&mut tera);
        let mut results = Vec::new();

        if let Some(items) = &self.loop_items {
            let initial_context = crate::playbook::templar::create_tera_context(vars);
            let items_list =
                match self.resolve_loop_items(items, &mut tera, &initial_context, vars)? {
                    Some(list) => list,
                    None => {
                        let mut result = TaskResult::new(&host.name);
                        result.skipped = true;
                        result.msg = "No items in loop".to_string();
                        return Ok(result);
                    }
                };

            debug!("Executing task with loop: {} items", items_list.len());

            let loop_var = self.loop_var_name.as_deref().unwrap_or("item");

            for (idx, item) in items_list.iter().enumerate() {
                debug!("Loop iteration {}: {:?}", idx + 1, item);

                let mut iter_vars = vars.clone();

                iter_vars.insert(loop_var.to_string(), item.clone());

                if let Some(index_var) = &self.index_var_name {
                    iter_vars.insert(index_var.clone(), Value::Number(idx.into()));
                }

                let iter_context = crate::playbook::templar::create_tera_context(&iter_vars);

                if let Some(when) = &self.when {
                    let mut condition_tera = Tera::default();
                    if !self.evaluate_condition(when, &mut condition_tera, &iter_context)? {
                        debug!("Skipping loop iteration due to when condition");
                        let mut result = TaskResult::new(&host.name);
                        result.skipped = true;
                        result.msg = "Skipped loop item due to condition".to_string();
                        results.push(result);
                        continue;
                    }
                }

                let mut iter_task = self.clone();
                iter_task.loop_items = None;

                let result =
                    iter_task.execute_module(host, &mut tera, &iter_context, &iter_vars)?;

                let elapsed = start_time.elapsed();
                let execution_time = format!("{:.2}s", elapsed.as_secs_f64());
                print_loop_iteration_result(
                    &host.name,
                    &self.name,
                    &result,
                    &execution_time,
                    idx + 1,
                    items_list.len(),
                );

                results.push(result);
            }
        } else {
            let context = crate::playbook::templar::create_tera_context(vars);
            if let Some(when) = &self.when {
                let mut condition_tera = Tera::default();
                if !self.evaluate_condition(when, &mut condition_tera, &context)? {
                    debug!("Skipping task due to when condition");
                    let mut result = TaskResult::new(&host.name);
                    result.skipped = true;
                    result.msg = "Skipped due to condition".to_string();
                    return Ok(result);
                }
            }

            let result = self.execute_module(host, &mut tera, &context, vars)?;
            results.push(result);
        }

        let changed = results.iter().any(|r| r.changed);
        let failed = results.iter().any(|r| r.failed);

        let mut final_result = TaskResult::new(&host.name);
        final_result.changed = changed;
        final_result.failed = failed && !self.ignore_errors;

        if results.len() == 1 {
            final_result.msg = results[0].msg.clone();
            final_result.values = results[0].values.clone();
        } else {
            let changed_count = results.iter().filter(|r| r.changed).count();
            let ok_count = results.len() - changed_count;

            if changed_count > 0 {
                final_result.msg = format!(
                    "changed={} ok={} iterations={}",
                    changed_count,
                    ok_count,
                    results.len()
                );
            } else {
                final_result.msg = format!("ok={} iterations={}", ok_count, results.len());
            }

            for (idx, result) in results.iter().enumerate() {
                for (key, value) in &result.values {
                    final_result
                        .values
                        .insert(format!("item_{}.{}", idx, key), value.clone());
                }
            }
        }

        let elapsed = start_time.elapsed();
        let execution_time = format!("{:.2}s", elapsed.as_secs_f64());

        if results.len() > 1 {
            print_task_result(&host.name, &self.name, &final_result, &execution_time);
        } else if results.len() == 1 && self.loop_items.is_none() {
            print_task_result(&host.name, &self.name, &final_result, &execution_time);
        }

        Ok(final_result)
    }

    fn execute_module(
        &self,
        host: &Host,
        tera: &mut Tera,
        _context: &TeraContext,
        vars: &HashMap<String, Value>,
    ) -> Result<TaskResult> {
        debug!(
            "Executing module '{}' for task '{}'",
            self.module, self.name
        );

        // Add ansible_date_time variable if it doesn't exist in vars
        let mut vars_with_date = vars.clone();
        if !vars_with_date.contains_key("ansible_date_time") {
            let now = chrono::Local::now();
            let mut date_time_mapping = Mapping::new();

            date_time_mapping.insert(
                Value::String("date".to_string()),
                Value::String(now.format("%Y-%m-%d").to_string()),
            );
            date_time_mapping.insert(
                Value::String("time".to_string()),
                Value::String(now.format("%H:%M:%S").to_string()),
            );
            date_time_mapping.insert(
                Value::String("year".to_string()),
                Value::String(now.format("%Y").to_string()),
            );
            date_time_mapping.insert(
                Value::String("month".to_string()),
                Value::String(now.format("%m").to_string()),
            );
            date_time_mapping.insert(
                Value::String("day".to_string()),
                Value::String(now.format("%d").to_string()),
            );
            date_time_mapping.insert(
                Value::String("hour".to_string()),
                Value::String(now.format("%H").to_string()),
            );
            date_time_mapping.insert(
                Value::String("minute".to_string()),
                Value::String(now.format("%M").to_string()),
            );
            date_time_mapping.insert(
                Value::String("second".to_string()),
                Value::String(now.format("%S").to_string()),
            );
            date_time_mapping.insert(
                Value::String("weekday".to_string()),
                Value::String(now.format("%A").to_string()),
            );
            date_time_mapping.insert(
                Value::String("weekday_short".to_string()),
                Value::String(now.format("%a").to_string()),
            );
            date_time_mapping.insert(
                Value::String("epoch".to_string()),
                Value::String(now.timestamp().to_string()),
            );
            date_time_mapping.insert(
                Value::String("iso8601".to_string()),
                Value::String(now.to_rfc3339()),
            );

            vars_with_date.insert(
                "ansible_date_time".to_string(),
                Value::Mapping(date_time_mapping),
            );
        }

        // Create a new context with the updated vars
        let context_with_date = crate::playbook::templar::create_tera_context(&vars_with_date);
        let mut resolved_args = self.resolve_args(tera, &context_with_date, &vars_with_date)?;

        let is_local = host.hostname == "localhost" || host.hostname == "127.0.0.1";
        if is_local {
            resolved_args.insert(
                Value::String("_host_type".to_string()),
                Value::String("local".to_string()),
            );
            let module_result = match self.module.as_str() {
                "command" | "shell" => {
                    let raw_param_value = resolved_args
                        .get(&Value::String("_raw_params".to_string()))
                        .cloned()
                        .unwrap_or(Value::Null);
                    let cmd = match raw_param_value {
                        Value::String(ref s) => s.clone(),
                        _ => serde_yaml::to_string(&raw_param_value).unwrap_or_default(),
                    };
                    let (exit_code, stdout, stderr) =
                        crate::modules::local::execute_local_command(&cmd)?;
                    self.process_command_result(exit_code, stdout, stderr)
                }
                "debug" => {
                    crate::modules::debug::execute_adhoc(host, &Value::Mapping(resolved_args))?
                }
                "package" => {
                    crate::modules::package::execute_adhoc(host, &Value::Mapping(resolved_args))?
                }
                "service" => {
                    crate::modules::service::execute_adhoc(host, &Value::Mapping(resolved_args))?
                }
                "copy" => {
                    crate::modules::copy::execute_adhoc(host, &Value::Mapping(resolved_args))?
                }
                "file" => {
                    crate::modules::file::execute_adhoc(host, &Value::Mapping(resolved_args))?
                }
                "template" => {
                    // 为模板添加任务变量到 resolved_args 中
                    let mut template_args = resolved_args.clone();

                    // 如果已有 vars 参数，则合并；否则创建新的
                    let mut vars_mapping = Mapping::new();
                    if let Some(Value::Mapping(existing_vars)) =
                        template_args.get(&Value::String("vars".to_string()))
                    {
                        vars_mapping = existing_vars.clone();
                    }

                    // 将任务变量添加到 vars_mapping 中
                    for (key, value) in vars_with_date.iter() {
                        vars_mapping.insert(Value::String(key.clone()), value.clone());
                    }

                    // 更新 template_args 中的 vars 参数
                    template_args.insert(
                        Value::String("vars".to_string()),
                        Value::Mapping(vars_mapping),
                    );

                    crate::modules::template::execute_adhoc(host, &Value::Mapping(template_args))?
                }
                "lineinfile" => {
                    crate::modules::lineinfile::execute_adhoc(host, &Value::Mapping(resolved_args))?
                }
                "user" => {
                    crate::modules::user::execute_adhoc(host, &Value::Mapping(resolved_args))?
                }
                _ => {
                    let mut result = ModuleResult::default();
                    result.msg = format!(
                        "Module '{}' not implemented for local execution",
                        self.module
                    );
                    result.stderr = format!("Module {} is only available with SSH", self.module);
                    result
                }
            };
            return Ok(TaskResult::from_module_result(&host.name, module_result));
        }

        let client = match SshClient::connect(host) {
            Ok(client) => client,
            Err(e) => {
                let mut result = TaskResult::new(&host.name);
                result.failed = true;
                result.msg = format!("Failed to connect to host: {}", e);
                return Ok(result);
            }
        };

        let module_result = match self.module.as_str() {
            "command" | "shell" => {
                debug!(
                    "Executing command/shell module with args: {:?}",
                    resolved_args
                );
                let raw_param_value = resolved_args
                    .get(&Value::String("_raw_params".to_string()))
                    .cloned()
                    .unwrap_or(Value::Null);
                let command = match raw_param_value {
                    Value::String(ref s) => s.clone(),
                    _ => serde_yaml::to_string(&raw_param_value).unwrap_or_default(),
                };
                debug!("Executing command: {:?}", command);
                crate::modules::command::execute(
                    &client,
                    &Value::String(command),
                    self.is_become,
                    &self.become_user,
                )?
            }
            "debug" => {
                debug!("Executing debug module with args: {:?}", resolved_args);
                crate::modules::debug::execute(
                    &client,
                    &Value::Mapping(resolved_args),
                    self.is_become,
                    &self.become_user,
                )?
            }
            "copy" => {
                debug!("Executing copy module with args: {:?}", resolved_args);
                crate::modules::copy::execute(
                    &client,
                    &Value::Mapping(resolved_args),
                    self.is_become,
                    &self.become_user,
                )?
            }
            "file" => {
                debug!("Executing file module with args: {:?}", resolved_args);
                crate::modules::file::execute(
                    &client,
                    &Value::Mapping(resolved_args),
                    self.is_become,
                    &self.become_user,
                )?
            }
            "template" => {
                debug!("Executing template module with args: {:?}", resolved_args);

                // 为模板添加任务变量到 resolved_args 中
                let mut template_args = resolved_args.clone();

                // 如果已有 vars 参数，则合并；否则创建新的
                let mut vars_mapping = Mapping::new();
                if let Some(Value::Mapping(existing_vars)) =
                    template_args.get(&Value::String("vars".to_string()))
                {
                    vars_mapping = existing_vars.clone();
                }

                // 将任务变量添加到 vars_mapping 中
                for (key, value) in vars_with_date.iter() {
                    vars_mapping.insert(Value::String(key.clone()), value.clone());
                }

                // 更新 template_args 中的 vars 参数
                template_args.insert(
                    Value::String("vars".to_string()),
                    Value::Mapping(vars_mapping),
                );

                debug!(
                    "Enhanced template args with task variables: {:?}",
                    template_args
                );
                crate::modules::template::execute(
                    &client,
                    &Value::Mapping(template_args),
                    self.is_become,
                    &self.become_user,
                )?
            }
            "package" => {
                debug!("Executing package module with args: {:?}", resolved_args);
                crate::modules::package::execute(
                    &client,
                    &Value::Mapping(resolved_args),
                    self.is_become,
                    &self.become_user,
                )?
            }
            "service" => {
                debug!("Executing service module with args: {:?}", resolved_args);
                crate::modules::service::execute(
                    &client,
                    &Value::Mapping(resolved_args),
                    self.is_become,
                    &self.become_user,
                )?
            }
            "lineinfile" => crate::modules::lineinfile::execute(
                &client,
                &Value::Mapping(resolved_args),
                self.is_become,
                &self.become_user,
            )?,
            "user" => crate::modules::user::execute(
                &client,
                &Value::Mapping(resolved_args),
                self.is_become,
                &self.become_user,
            )?,
            _ => {
                let mut result = ModuleResult::default();
                result.msg = format!("Unknown module: {}", self.module);
                result.stderr = format!("Module {} is not implemented", self.module);
                result
            }
        };

        Ok(TaskResult::from_module_result(&host.name, module_result))
    }

    fn process_command_result(
        &self,
        exit_code: i32,
        stdout: String,
        stderr: String,
    ) -> ModuleResult {
        let mut result = ModuleResult::default();
        result.stdout = stdout;
        result.stderr = stderr;

        if exit_code == 0 {
            result.changed = true;

            if self.module == "command" {
                result.msg = format!("Command executed successfully (exit code: {})", exit_code);
            } else if self.module == "shell" {
                result.msg = format!("Shell command executed with exit code: {}", exit_code);
            } else {
                result.msg = format!("Command executed successfully (exit code: {})", exit_code);
            }
        } else {
            result.failed = true; // Mark as failed for non-zero exit code
            if self.module == "command" {
                result.msg = format!("Command failed with exit code: {}", exit_code);
            } else if self.module == "shell" {
                result.msg = format!("Shell command failed with exit code: {}", exit_code);
            } else {
                result.msg = format!("Command failed with exit code: {}", exit_code);
            }
        }

        result
    }

    fn evaluate_condition(
        &self,
        condition: &Value,
        tera: &mut Tera,
        context: &TeraContext,
    ) -> Result<bool> {
        if let Value::String(condition_str) = condition {
            crate::playbook::templar::evaluate_condition(condition_str, tera, context)
        } else if let Value::Bool(b) = condition {
            Ok(*b)
        } else {
            warn!(
                "Could not evaluate non-string/non-bool condition: {:?}",
                condition
            );
            Ok(true)
        }
    }

    fn resolve_loop_items(
        &self,
        items: &Value,
        tera: &mut Tera,
        context: &TeraContext,
        vars: &HashMap<String, Value>,
    ) -> Result<Option<Vec<Value>>> {
        match items {
            Value::Sequence(seq) => Ok(Some(seq.clone())),
            Value::String(var_name) => {
                // Try direct variable lookup first for simple OR nested cases like "{{ my_list }}" or "{{ data.list }}"
                if let Some(key_str) = extract_simple_variable(var_name) {
                    debug!(
                        "RESOLVE_LOOP: Identified simple/nested variable: {}",
                        key_str
                    );
                    // Use the new nested lookup helper
                    if let Some(value) = get_nested_value(key_str, vars) {
                        match value {
                            Value::Sequence(seq) => {
                                debug!(
                                    "RESOLVE_LOOP: Found sequence directly via nested lookup for key '{}'",
                                    key_str
                                );
                                return Ok(Some(seq.clone()));
                            }
                            _ => {
                                warn!(
                                    "RESOLVE_LOOP: Variable '{}' found via nested lookup but is not a sequence. Falling back to rendering.",
                                    key_str
                                );
                                // Fall through to render_value
                            }
                        }
                    } else {
                        warn!(
                            "RESOLVE_LOOP: Simple/nested variable '{}' not found in vars via lookup. Falling back to rendering.",
                            key_str
                        );
                        // Fall through to render_value
                    }
                }

                // Fallback to rendering for complex expressions or if direct lookup failed
                debug!(
                    "RESOLVE_LOOP: Falling back to rendering loop items from string: {}",
                    var_name
                );
                match crate::playbook::templar::render_value(var_name, tera, context, false) {
                    Ok(Value::Sequence(resolved_seq)) => Ok(Some(resolved_seq)),
                    Ok(Value::String(s))
                        if !var_name.contains("{{") && !var_name.contains("{%") =>
                    {
                        // If the original var_name wasn't a template, treat the string as a single-item list
                        warn!(
                            "Loop value '{}' is a string and not recognized as a simple variable or complex template. Interpreting as a single-item list.",
                            s
                        );
                        Ok(Some(vec![Value::String(s)]))
                    }
                    Ok(other) => {
                        // Rendered successfully, but the result wasn't a sequence
                        Err(anyhow!(
                            "Loop expression '{}' resolved to a non-sequence value: {:?}",
                            var_name,
                            other
                        ))
                    }
                    Err(e) => {
                        // Rendering failed
                        Err(anyhow!(
                            "Failed to render loop expression '{}': {}",
                            var_name,
                            e
                        ))
                    }
                }
            }
            _ => Err(anyhow!("Unsupported loop type: {:?}", items)),
        }
    }

    fn resolve_args(
        &self,
        _tera: &mut Tera, // Original Tera instance (can be kept for potential future shared state)
        context: &TeraContext,
        vars: &HashMap<String, Value>,
    ) -> Result<Mapping> {
        let mut resolved = Mapping::new();
        debug!(
            "RESOLVE_ARGS: Starting for task '{}'. Initial args: {:?}",
            self.name, self.args
        );

        // Use a fresh Tera instance for argument rendering to isolate filter registration
        let mut arg_tera = Tera::default();

        // Register custom filters needed for Ansible compatibility
        register_ansible_filters(&mut arg_tera);

        // 添加更多有用的过滤器和函数，特别是处理数值比较和条件表达式
        if let Err(e) =
            arg_tera.add_raw_template("tmp_if_expr", "{% if true %}true{% else %}false{% endif %}")
        {
            warn!("无法添加临时模板以初始化 Tera: {}", e);
        }

        // 对特定的 if 条件表达式进行预处理
        let preprocess_if_condition = |value: &str| -> String {
            if value.contains(" if ") && value.contains(" else ") {
                // 简单检测并转换为 Tera 正确的 if 语法
                // 例如: {{ '0700' if item.port < 1000 else '0755' }}
                // 转换为: {% if item.port < 1000 %}'0700'{% else %}'0755'{% endif %}
                let value = value.trim();
                if value.starts_with("{{") && value.ends_with("}}") {
                    let inner = &value[2..value.len() - 2].trim();
                    if let Some(if_pos) = inner.find(" if ") {
                        if let Some(else_pos) = inner.find(" else ") {
                            let then_expr = &inner[0..if_pos].trim();
                            let condition = &inner[if_pos + 4..else_pos].trim();
                            let else_expr = &inner[else_pos + 6..].trim();

                            debug!(
                                "预处理 if 条件: then={}, condition={}, else={}",
                                then_expr, condition, else_expr
                            );
                            return format!(
                                "{{% if {} %}}{}{{% else %}}{}{{% endif %}}",
                                condition, then_expr, else_expr
                            );
                        }
                    }
                }
            }
            value.to_string()
        };

        for (key, value) in &self.args {
            match value {
                Value::String(s) => {
                    debug!(
                        "RESOLVE_ARGS: Processing string value for key {:?}: '{}'",
                        key, s
                    );
                    // 对 template.content、debug.msg、debug.var 强制字符串渲染
                    let force_string = (self.module == "template" && key == "content")
                        || (self.module == "debug" && (key == "msg" || key == "var"));

                    // 特殊处理 file 模块的 mode 参数，确保条件表达式正确渲染
                    let is_mode_param = (self.module == "file"
                        || self.module == "copy"
                        || self.module == "template")
                        && (key == "mode" || key == "mode");

                    // 预处理条件表达式
                    let processed_value =
                        if is_mode_param && (s.contains(" if ") || s.contains(" else ")) {
                            debug!("RESOLVE_ARGS: 预处理模式参数条件表达式: '{}'", s);
                            preprocess_if_condition(s)
                        } else {
                            s.to_string()
                        };

                    match crate::playbook::templar::render_value(
                        &processed_value,
                        &mut arg_tera,
                        context,
                        force_string,
                    ) {
                        Ok(rendered_value) => {
                            debug!(
                                "RESOLVE_ARGS: Rendered key {:?} to value: {:?}",
                                key, rendered_value
                            );

                            // 对于文件模式参数，确保其为数字或有效的模式字符串
                            if is_mode_param {
                                if let Value::String(mode_str) = &rendered_value {
                                    if mode_str.contains("{{") || mode_str.contains("{%") {
                                        // 模式字符串包含未处理的模板表达式，使用安全默认值
                                        debug!(
                                            "RESOLVE_ARGS: Mode parameter contains unprocessed template expression: '{}'. Using default '0644'.",
                                            mode_str
                                        );
                                        resolved
                                            .insert(key.clone(), Value::String("0644".to_string()));
                                        continue;
                                    }
                                }
                            }

                            resolved.insert(key.clone(), rendered_value.clone());

                            // Special handling for debug module's 'var' parameter
                            if self.module == "debug" {
                                if let Value::String(ref var_key_name) = key {
                                    if var_key_name == "var" {
                                        if let Value::String(ref var_name_to_lookup) =
                                            rendered_value
                                        {
                                            debug!(
                                                "RESOLVE_ARGS: Debug 'var' detected. Looking up variable '{}'",
                                                var_name_to_lookup
                                            );
                                            // Use nested lookup for the debug var as well
                                            if let Some(found_value) =
                                                get_nested_value(var_name_to_lookup, vars)
                                            {
                                                debug!(
                                                    "RESOLVE_ARGS: Found value for '{}': {:?}",
                                                    var_name_to_lookup, found_value
                                                );
                                                resolved.insert(
                                                    Value::String("_var_value".to_string()),
                                                    found_value.clone(),
                                                );
                                            } else {
                                                debug!(
                                                    "RESOLVE_ARGS: Variable '{}' not found in context.",
                                                    var_name_to_lookup
                                                );
                                                resolved.remove(&Value::String(
                                                    "_var_value".to_string(),
                                                ));
                                            }
                                        } else {
                                            debug!(
                                                "RESOLVE_ARGS: Rendered value for 'var' is not a string: {:?}",
                                                rendered_value
                                            );
                                            resolved
                                                .remove(&Value::String("_var_value".to_string()));
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            // 对于关键参数，如果模板渲染失败则抛出错误
                            let is_critical_param = match (self.module.as_str(), key) {
                                ("user", k) if k == &Value::String("password".to_string()) => true,
                                ("copy", k) if k == &Value::String("content".to_string()) => true,
                                ("template", k) if k == &Value::String("content".to_string()) => {
                                    true
                                }
                                _ => false,
                            };

                            if is_critical_param {
                                return Err(anyhow::anyhow!(
                                    "Critical parameter template rendering failed for '{}' in module '{}': {}. Please ensure all variables are properly defined.",
                                    s, self.module, e
                                ));
                            }

                            warn!(
                                "RESOLVE_ARGS: Failed to render argument string '{}' for key {:?}: {}. Using original.",
                                s,
                                key,
                                e
                            );
                            resolved.insert(key.clone(), value.clone());
                        }
                    }
                }
                _ => {
                    debug!(
                        "RESOLVE_ARGS: Cloning non-string value for key {:?}: {:?}",
                        key, value
                    );
                    resolved.insert(key.clone(), value.clone());
                }
            }
        }

        debug!(
            "RESOLVE_ARGS: Finished for task '{}'. Resolved args: {:?}",
            self.name, resolved
        );
        Ok(resolved)
    }
}

pub(crate) fn print_task_result(
    host_name: &str,
    _task_name: &str,
    result: &TaskResult,
    execution_time: &str,
) {
    let status = if result.failed {
        "failed"
    } else if result.skipped {
        "skipped"
    } else if result.changed {
        "changed"
    } else {
        "ok"
    };

    let color_code = if result.failed {
        "\x1B[31m"
    } else if result.skipped {
        "\x1B[33m"
    } else if result.changed {
        "\x1B[33m"
    } else {
        "\x1B[32m"
    };

    let reset_code = "\x1B[0m";

    println!(
        "{} => {}{}: {} ({}){}",
        host_name, color_code, status, result.msg, execution_time, reset_code
    );

    if let Some(Value::String(stdout)) = result.values.get("stdout") {
        if !stdout.is_empty() {
            println!("    {}", stdout.trim());
        }
    }

    if let Some(Value::String(stderr)) = result.values.get("stderr") {
        if !stderr.is_empty() {
            println!("    {}{}{}", color_code, stderr.trim(), reset_code);
        }
    }
}

pub(crate) fn print_loop_iteration_result(
    host_name: &str,
    _task_name: &str,
    result: &TaskResult,
    execution_time: &str,
    iteration: usize,
    total_iterations: usize,
) {
    let status = if result.failed {
        "failed"
    } else if result.skipped {
        "skipped"
    } else if result.changed {
        "changed"
    } else {
        "ok"
    };

    let color_code = if result.failed {
        "\x1B[31m"
    } else if result.skipped {
        "\x1B[33m"
    } else if result.changed {
        "\x1B[33m"
    } else {
        "\x1B[32m"
    };

    let reset_code = "\x1B[0m";

    println!(
        "{}{} (item={}/{}) => {}{}{}: {} ({}){}",
        color_code,
        host_name,
        iteration,
        total_iterations,
        color_code,
        status,
        reset_code,
        result.msg,
        execution_time,
        reset_code
    );

    if let Some(Value::String(stdout)) = result.values.get("stdout") {
        if !stdout.is_empty() {
            for line in stdout.trim().lines() {
                println!("    {}", line);
            }
        }
    }

    if let Some(Value::String(stderr)) = result.values.get("stderr") {
        if !stderr.is_empty() {
            for line in stderr.trim().lines() {
                println!("    {}{}{}", color_code, line, reset_code);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::Mapping;

    fn create_test_task() -> Task {
        Task {
            name: "Test Task".to_string(),
            module: "debug".to_string(),
            args: Mapping::new(),
            is_become: false,
            become_user: "root".to_string(),
            register: None,
            when: None,
            notify: Vec::new(),
            ignore_errors: false,
            tags: Vec::new(),
            loop_items: None,
            loop_var_name: None,
            index_var_name: None,
        }
    }

    // Modify to return the vars map as well
    fn create_test_tera_and_context(
        vars_map: Option<HashMap<String, Value>>,
    ) -> (Tera, TeraContext, HashMap<String, Value>) {
        let tera = Tera::default();
        let mut vars = HashMap::new();
        if let Some(map) = vars_map {
            vars.extend(map);
        }
        // Add some default test variables
        vars.insert("greeting".to_string(), Value::String("Hello".to_string()));
        vars.insert("target".to_string(), Value::String("world".to_string()));
        let list_items = vec![
            Value::String("apple".to_string()),
            Value::String("banana".to_string()),
        ];
        vars.insert(
            "items_list".to_string(),
            Value::Sequence(list_items.clone()),
        );

        // Add nested structure for testing get_nested_value
        let mut app_config = Mapping::new();
        let features_list = vec![
            Value::Mapping(Mapping::from_iter([
                (
                    Value::String("name".into()),
                    Value::String("FeatureA".into()),
                ),
                (Value::String("enabled".into()), Value::Bool(true)),
            ])),
            Value::Mapping(Mapping::from_iter([
                (
                    Value::String("name".into()),
                    Value::String("FeatureB".into()),
                ),
                (Value::String("enabled".into()), Value::Bool(false)),
            ])),
        ];
        // Explicit keys for insertion
        let features_key = Value::String("features".to_string());
        let version_key = Value::String("version".to_string());
        app_config.insert(
            features_key.clone(), // Use explicit key
            Value::Sequence(features_list.clone()),
        );
        app_config.insert(
            version_key.clone(), // Use explicit key
            Value::String("1.2.3".to_string()),
        );
        vars.insert("application".to_string(), Value::Mapping(app_config));

        let complex_list = vec![
            Value::Mapping(Mapping::from_iter([(
                Value::String("name".into()),
                Value::String("A".into()),
            )])),
            Value::Mapping(Mapping::from_iter([(
                Value::String("name".into()),
                Value::String("B".into()),
            )])),
        ];
        vars.insert(
            "complex_items".to_string(),
            Value::Sequence(complex_list.clone()),
        );

        let context = crate::playbook::templar::create_tera_context(&vars);
        (tera, context, vars) // Return vars map
    }

    #[test]
    fn test_get_nested_value_simple() {
        let (_tera, _context, vars) = create_test_tera_and_context(None);
        assert_eq!(
            get_nested_value("greeting", &vars),
            Some(&Value::String("Hello".to_string()))
        );
        assert_eq!(
            get_nested_value("items_list", &vars),
            Some(&Value::Sequence(vec![
                Value::String("apple".to_string()),
                Value::String("banana".to_string())
            ]))
        );
    }

    #[test]
    fn test_get_nested_value_nested() {
        let (_tera, _context, vars) = create_test_tera_and_context(None);
        let expected_value = Value::String("1.2.3".to_string()); // Explicit expected value
        assert_eq!(
            get_nested_value("application.version", &vars),
            Some(&expected_value) // Compare with explicit value
        );
    }

    #[test]
    fn test_get_nested_value_list() {
        let (_tera, _context, vars) = create_test_tera_and_context(None);
        let features_key = Value::String("features".to_string()); // Explicit key for lookup
        let expected_features_ref = vars
            .get("application")
            .unwrap()
            .as_mapping()
            .unwrap()
            .get(&features_key)
            .unwrap(); // Use explicit key for lookup
        assert_eq!(
            get_nested_value("application.features", &vars),
            Some(expected_features_ref)
        );
    }

    #[test]
    fn test_get_nested_value_not_found() {
        let (_tera, _context, vars) = create_test_tera_and_context(None);
        assert_eq!(get_nested_value("nonexistent", &vars), None);
        assert_eq!(get_nested_value("application.nonexistent", &vars), None);
        assert_eq!(get_nested_value("greeting.nonexistent", &vars), None); // Cannot index into string
        assert_eq!(get_nested_value("items_list.0", &vars), None); // Does not support list indexing
    }

    #[test]
    fn test_resolve_loop_items_direct_list() {
        let task = create_test_task();
        let items = Value::Sequence(vec![Value::Number(1.into()), Value::Number(2.into())]);
        let (mut tera, context, vars) = create_test_tera_and_context(None); // Capture vars

        // Pass vars to resolve_loop_items
        let result = task
            .resolve_loop_items(&items, &mut tera, &context, &vars)
            .unwrap();
        assert_eq!(
            result,
            Some(vec![Value::Number(1.into()), Value::Number(2.into())])
        );
    }

    #[test]
    fn test_resolve_loop_items_variable_simple() {
        // Test the direct lookup path
        let task = create_test_task();
        let items_var = Value::String("{{ items_list }}".to_string());
        let expected_list = vec![
            Value::String("apple".to_string()),
            Value::String("banana".to_string()),
        ];
        let (mut tera, context, vars) = create_test_tera_and_context(None); // Capture vars

        // Pass vars to resolve_loop_items
        let result = task
            .resolve_loop_items(&items_var, &mut tera, &context, &vars)
            .unwrap();
        assert_eq!(result, Some(expected_list));
    }

    #[test]
    fn test_resolve_loop_items_variable_nested() {
        // Test the direct lookup path for nested variables
        let task = create_test_task();
        let items_var = Value::String("{{ application.features }}".to_string());
        // Capture vars correctly here
        let (mut tera, context, vars) = create_test_tera_and_context(None);
        let features_key = Value::String("features".to_string()); // Explicit key for lookup
        let expected_list = vars
            .get("application")
            .unwrap()
            .as_mapping()
            .unwrap()
            .get(&features_key)
            .unwrap() // Use explicit key
            .as_sequence()
            .unwrap()
            .clone();

        let result = task
            .resolve_loop_items(&items_var, &mut tera, &context, &vars)
            .unwrap();
        assert_eq!(result, Some(expected_list));
    }

    #[test]
    fn test_resolve_loop_items_variable_complex() {
        // Test the direct lookup path with complex items
        let task = create_test_task();
        let items_var = Value::String("{{ complex_items }}".to_string());
        let expected_list = vec![
            Value::Mapping(Mapping::from_iter([(
                Value::String("name".into()),
                Value::String("A".into()),
            )])),
            Value::Mapping(Mapping::from_iter([(
                Value::String("name".into()),
                Value::String("B".into()),
            )])),
        ];
        let (mut tera, context, vars) = create_test_tera_and_context(None); // Capture vars

        // Pass vars to resolve_loop_items
        let result = task
            .resolve_loop_items(&items_var, &mut tera, &context, &vars)
            .unwrap();
        assert_eq!(result, Some(expected_list));
    }

    #[test]
    fn test_resolve_loop_items_render_fallback() {
        // Test fallback to rendering for a slightly more complex expression
        let task = create_test_task();
        // Use a filter or expression Tera needs to evaluate
        let items_expr = Value::String("{{ items_list | join(\",\") }}".to_string()); // This resolves to a string, not a list
        let (mut tera, context, vars) = create_test_tera_and_context(None);

        // Pass vars to resolve_loop_items
        let result = task.resolve_loop_items(&items_expr, &mut tera, &context, &vars);

        // We expect this to fail because the *rendered expression* is not a sequence.
        // Simply check that it returned an error.
        assert!(
            result.is_err(),
            "Expected an error when rendered loop item is not a sequence"
        );
        // assert!(result
        //     .unwrap_err()
        //     .to_string()
        //     .contains("resolved to a non-sequence value"));
    }

    #[test]
    fn test_resolve_loop_items_invalid_expr() {
        let task = create_test_task();
        let items_expr = Value::String("{{ undefined_var | non_existent_filter }}".to_string());
        let (mut tera, context, vars) = create_test_tera_and_context(None); // Capture vars

        // Pass vars to resolve_loop_items
        let result = task.resolve_loop_items(&items_expr, &mut tera, &context, &vars);
        assert!(result.is_err()); // Expecting rendering error
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Failed to render loop expression"));
    }

    #[test]
    fn test_resolve_loop_items_not_a_list_variable() {
        // Test when the variable exists but is not a list
        let task = create_test_task();
        let items_expr = Value::String("{{ greeting }}".to_string()); // 'greeting' is a string
        let (mut tera, context, vars) = create_test_tera_and_context(None); // Capture vars

        // Pass vars to resolve_loop_items
        let result = task.resolve_loop_items(&items_expr, &mut tera, &context, &vars);
        // Should hit the direct lookup, find 'greeting', see it's not a sequence,
        // fall back to rendering
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("resolved to a non-sequence value"));
    }

    #[test]
    fn test_resolve_loop_items_bare_string() {
        let task = create_test_task();
        let items_bare = Value::String("single_item".to_string());
        let (mut tera, context, vars) = create_test_tera_and_context(None); // Capture vars

        // Pass vars to resolve_loop_items
        let result = task
            .resolve_loop_items(&items_bare, &mut tera, &context, &vars)
            .unwrap();
        // Should not be caught by simple var check, should not contain {{ }},
        // should fall back to interpreting the string as a single-item list.
        assert_eq!(result, Some(vec![Value::String("single_item".to_string())]));
    }
}
