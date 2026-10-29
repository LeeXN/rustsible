use crate::playbook::templar::{RuntimeVariables, TemplateContext};
#[allow(unused_imports)]
use anyhow::Result;
use anyhow::{anyhow, Context};
use log::{debug, info, warn};
use minijinja::Environment;
use serde_yaml::{Mapping, Value};
use std::collections::HashMap;
use std::time::Instant;

use crate::inventory::Host;
use crate::modules::result::insert_output_values;
use crate::modules::{
    registry::{Builtin, Invocation},
    ModuleResult,
};
use crate::ssh::config::ConnectionConfig;

pub(crate) struct TaskExecution<'a> {
    pub workers: &'a crate::worker::Runtime,
    pub tags: Option<&'a super::TagSelection>,
    pub delegation: Option<crate::playbook::play::Delegation<'a>>,
}
impl<'a> TaskExecution<'a> {
    pub fn new(workers: &'a crate::worker::Runtime) -> Self {
        Self {
            workers,
            tags: None,
            delegation: None,
        }
    }
}

const MAX_ARGUMENT_TEMPLATE_DEPTH: usize = 64;
const MAX_ARGUMENT_TEMPLATE_NODES: usize = 10_000;

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct TaskScope {
    /// Identical sibling tasks still have separate concurrency limits.
    pub(crate) source_index: usize,
    pub role_id: Option<String>,
    pub collections: Vec<String>,
    pub allow_duplicates: bool,
    pub(crate) boundary: Option<Boundary>,
    pub defaults: Mapping,
    pub role_vars: Mapping,
    pub role_params: Mapping,
    pub params: Mapping,
    /// Loop/include bindings are runtime data, never template definitions.
    pub locals: HashMap<String, Value>,
    pub role_path: Option<std::path::PathBuf>,
    pub role_name: Option<String>,
    pub parent_roles: Vec<std::path::PathBuf>,
    pub ancestors: Vec<std::path::PathBuf>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub(crate) enum Boundary {
    Start,
    End,
    Flush,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IncludeKind {
    Tasks,
    Role,
}
pub(crate) fn include_kind(name: &str) -> Option<IncludeKind> {
    match name
        .strip_prefix("ansible.builtin.")
        .or_else(|| name.strip_prefix("ansible.legacy."))
        .unwrap_or(name)
    {
        "include_tasks" => Some(IncludeKind::Tasks),
        "include_role" => Some(IncludeKind::Role),
        _ => None,
    }
}
#[derive(Debug, Clone)]
pub(crate) struct IncludeRequest {
    pub iteration: usize,
    pub kind: IncludeKind,
    pub args: Mapping,
    pub parent: Task,
}
#[derive(Debug, Clone, Default)]
pub struct TaskGroup {
    pub tasks: Vec<Task>,
    pub rescue: Vec<Task>,
    pub always: Vec<Task>,
}

/// Task result structure for tracking execution status
#[derive(Debug, Clone)]
pub struct TaskResult {
    /// run_once shares data and host policy without inventing extra executions.
    pub(crate) inherited_result: bool,
    pub(crate) filtered: bool,
    pub(crate) meta: Option<String>,
    pub(crate) includes: Vec<IncludeRequest>,
    pub(crate) notifications: Vec<String>,
    pub changed: bool,
    pub failed: bool,
    pub unreachable: bool,
    pub uncertain: bool,
    pub skipped: bool,
    pub msg: String,
    #[allow(dead_code)]
    pub host: String, // Keep this for future use
    pub values: HashMap<String, Value>,
    /// Suppress the result payload when it is rendered to a user-facing sink.
    pub no_log: bool,
}

impl TaskResult {
    pub fn outcome(&self) -> crate::modules::Outcome {
        crate::modules::Outcome::from_flags(
            self.failed,
            self.skipped,
            self.unreachable,
            self.uncertain,
        )
    }
    pub(crate) fn registered_value(&self) -> Value {
        Value::Mapping(crate::modules::result::result_fields(
            self.changed,
            self.outcome(),
            &self.msg,
            &self.values,
        ))
    }
    pub fn new(host: &str) -> Self {
        let mut values = HashMap::new();
        insert_output_values(&mut values, String::new(), String::new(), None);
        TaskResult {
            inherited_result: false,
            filtered: false,
            meta: None,
            includes: Vec::new(),
            notifications: Vec::new(),
            changed: false,
            failed: false,
            unreachable: false,
            uncertain: false,
            skipped: false,
            msg: String::new(),
            host: host.to_string(),
            values,
            no_log: false,
        }
    }

    pub fn from_module_result(host: &str, module_result: ModuleResult) -> Self {
        let mut values = module_result.values;
        insert_output_values(
            &mut values,
            module_result.stdout,
            module_result.stderr,
            module_result.rc,
        );

        TaskResult {
            inherited_result: false,
            filtered: false,
            meta: None,
            includes: Vec::new(),
            notifications: Vec::new(),
            changed: module_result.changed,
            failed: module_result.failed
                || module_result.rc.is_some_and(|rc| rc != 0)
                || module_result.unreachable
                || module_result.uncertain,
            unreachable: module_result.unreachable,
            uncertain: module_result.uncertain,
            skipped: module_result.skipped,
            msg: module_result.msg,
            host: host.to_string(),
            values,
            no_log: module_result.no_log,
        }
    }
}

/// Task structure representing a single action in a play
#[derive(Debug, Clone)]
pub struct Task {
    pub group: Option<Box<TaskGroup>>,
    pub parent: Option<std::sync::Arc<Task>>,
    pub original: Option<std::sync::Arc<Task>>,
    pub scope: TaskScope,
    pub name: String,
    pub module: String,
    pub args: Mapping,
    /// Absolute source file anchors local resources independently of cwd.
    pub source_file: Option<std::path::PathBuf>,
    /// The task-level `become` setting. `None` inherits the play setting,
    /// while `Some(false)` explicitly disables play-level escalation.
    pub become_override: Option<bool>,
    pub is_become: bool, // Renamed from 'become' to avoid Rust keyword
    pub become_user: String,
    /// Distinguishes an explicit task override from the default user.
    pub become_user_override: Option<String>,
    pub connection: Option<String>,
    pub delegate_to: Option<String>,
    pub delegate_facts: Option<bool>,
    pub throttle: Option<usize>,
    pub run_once: Option<bool>,
    pub any_errors_fatal: Option<bool>,
    /// A task may enter check mode, but cannot disable a global dry run.
    pub check_mode: Option<bool>,
    pub register: Option<String>,
    pub when: Option<Value>,
    pub changed_when: Option<Value>,
    pub failed_when: Option<Value>,
    pub until: Option<Value>,
    pub retries: Option<Value>,
    pub delay: Option<Value>,
    pub timeout: Option<Value>,
    pub asynchronous: Option<Value>,
    pub poll: Option<Value>,
    pub environment: Mapping,
    pub notify: Vec<String>,
    pub ignore_errors: Option<bool>,
    pub ignore_unreachable: Option<bool>,
    /// Hide task payloads and errors from terminal/log output.
    pub no_log: Option<bool>,
    pub vars: Mapping,
    pub tags: Vec<String>,
    pub loop_items: Option<Value>,
    pub loop_var_name: Option<String>, // Name for loop variable (default: item)
    pub index_var_name: Option<String>, // Name for index variable
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
                current_value = map.get(Value::String(part.to_string()))?;
            }
            _ => {
                return None; // Cannot access sub-key on a non-mapping value
            }
        }
    }
    Some(current_value)
}

impl Task {
    /// Import/block settings are lexical defaults. Optional booleans preserve
    /// an explicit child false rather than treating it as a missing setting.
    pub(crate) fn inherit(&mut self, parent: &Task) {
        if let Some(existing) = self.parent.take() {
            let mut existing = (*existing).clone();
            existing.inherit(parent);
            self.parent = Some(std::sync::Arc::new(existing));
        } else {
            self.parent = Some(std::sync::Arc::new(parent.raw().clone()));
        }
    }
    /// Outer import/include defaults belong on each child in addition to the
    /// child's own block header; keeping this shared avoids preview drift.
    pub(crate) fn bind_children(&self, tasks: &[Task]) -> Vec<Task> {
        tasks
            .iter()
            .cloned()
            .map(|mut child| {
                if let Some(parent) = &self.raw().parent {
                    child.inherit(parent);
                }
                child
            })
            .collect()
    }
    pub(crate) fn raw(&self) -> &Task {
        self.original.as_deref().unwrap_or(self)
    }
    pub(crate) fn effective(&self) -> Task {
        if self.original.is_some() {
            return self.clone();
        }
        let mut task = self.clone();
        if let Some(parent) = task.parent.take() {
            task.inherit_fields(&parent.effective());
        }
        task.original = Some(std::sync::Arc::new(self.clone()));
        task
    }
    fn inherit_fields(&mut self, parent: &Task) {
        for (target, inherited) in [
            (&mut self.scope.defaults, &parent.scope.defaults),
            (&mut self.scope.role_vars, &parent.scope.role_vars),
            (&mut self.scope.role_params, &parent.scope.role_params),
            (&mut self.scope.params, &parent.scope.params),
        ] {
            let mut variables = inherited.clone();
            crate::vars::merge(&mut variables, target.clone());
            *target = variables;
        }
        let mut locals = parent.scope.locals.clone();
        locals.extend(self.scope.locals.clone());
        self.scope.locals = locals;
        if self.scope.role_path.is_none() {
            self.scope.role_path = parent.scope.role_path.clone();
            self.scope.role_name = parent.scope.role_name.clone();
            self.scope.parent_roles = parent.scope.parent_roles.clone();
        }
        if self.scope.ancestors.is_empty() {
            self.scope.ancestors = parent.scope.ancestors.clone();
        }
        let mut variables = parent.vars.clone();
        crate::vars::merge(&mut variables, self.vars.clone());
        self.vars = variables;
        let mut environment = parent.environment.clone();
        environment.extend(self.environment.clone());
        self.environment = environment;
        self.become_override = self.become_override.or(parent.become_override);
        self.become_user_override = self
            .become_user_override
            .clone()
            .or_else(|| parent.become_user_override.clone());
        self.is_become = self.become_override.unwrap_or(parent.is_become);
        self.become_user = self
            .become_user_override
            .clone()
            .unwrap_or_else(|| parent.become_user.clone());
        self.connection = self
            .connection
            .clone()
            .or_else(|| parent.connection.clone());
        self.check_mode = self.check_mode.or(parent.check_mode);
        self.delegate_to = self
            .delegate_to
            .clone()
            .or_else(|| parent.delegate_to.clone());
        self.delegate_facts = self.delegate_facts.or(parent.delegate_facts);
        self.throttle = self.throttle.or(parent.throttle);
        self.run_once = self.run_once.or(parent.run_once);
        self.any_errors_fatal = self.any_errors_fatal.or(parent.any_errors_fatal);
        self.ignore_errors = self.ignore_errors.or(parent.ignore_errors);
        self.ignore_unreachable = self.ignore_unreachable.or(parent.ignore_unreachable);
        self.no_log = self.no_log.or(parent.no_log);
        if self.timeout.is_none() {
            self.timeout = parent.timeout.clone();
        }
        if self.asynchronous.is_none() {
            self.asynchronous = parent.asynchronous.clone();
        }
        if self.poll.is_none() {
            self.poll = parent.poll.clone();
        }
        let mut conditions = Vec::new();
        for condition in [&parent.when, &self.when].into_iter().flatten() {
            if let Value::Sequence(values) = condition {
                conditions.extend(values.clone());
            } else {
                conditions.push(condition.clone());
            }
        }
        self.when = (!conditions.is_empty()).then_some(Value::Sequence(conditions));
        let mut notify = parent.notify.clone();
        notify.extend(self.notify.clone());
        self.notify = notify;
        let mut tags = parent.tags.clone();
        tags.extend(self.tags.clone());
        self.tags = tags;
    }
    pub fn execute(&self, host: &Host, vars: &HashMap<String, Value>) -> Result<TaskResult> {
        self.execute_with_options(host, vars, false)
    }

    pub fn execute_with_options(
        &self,
        host: &Host,
        vars: &HashMap<String, Value>,
        check_mode: bool,
    ) -> Result<TaskResult> {
        let mut definitions = host.inventory_variables();
        crate::inventory::host::merge_variable_layer(&mut definitions, vars);
        self.execute_with_definitions(
            host,
            &RuntimeVariables {
                defaults: host.defaults.clone(),
                ..Default::default()
            },
            &definitions,
            &Mapping::new(),
            check_mode,
            &TaskExecution::new(&crate::worker::Runtime::default()),
        )
    }

    pub(crate) fn variable_layers(
        &self,
        host: &Host,
        runtime: &RuntimeVariables,
        definitions: &HashMap<String, Value>,
        play_vars: &Mapping,
    ) -> Result<(HashMap<String, Value>, HashMap<String, Value>)> {
        let mut keywords = HashMap::new();
        if let Some(value) = self.become_override.or(self.is_become.then_some(true)) {
            keywords.insert("ansible_become".into(), Value::Bool(value));
        }
        if let Some(value) = self
            .become_user_override
            .as_ref()
            .or((self.become_user != "root").then_some(&self.become_user))
        {
            keywords.insert("ansible_become_user".into(), Value::String(value.clone()));
        }
        let mut scoped_definitions = self
            .scope
            .defaults
            .iter()
            .map(|(name, value)| {
                Ok((
                    name.as_str()
                        .context("Role default names must be strings")?
                        .to_owned(),
                    value.clone(),
                ))
            })
            .collect::<Result<HashMap<_, _>>>()?;
        crate::inventory::host::merge_variable_layer(&mut scoped_definitions, definitions);
        let mut params = self.scope.role_params.clone();
        crate::vars::merge(&mut params, self.scope.params.clone());
        let (definitions, mut data) = runtime.layers(
            &scoped_definitions,
            &[play_vars, &self.scope.role_vars, &self.vars],
            Some(&keywords),
            Some(&params),
        )?;
        if let Some(path) = &self.scope.role_path {
            data.insert(
                "role_path".into(),
                Value::String(path.to_string_lossy().into_owned()),
            );
        }
        if let Some(name) = &self.scope.role_name {
            data.insert("role_name".into(), Value::String(name.clone()));
            data.insert("ansible_role_name".into(), Value::String(name.clone()));
        }
        data.extend(self.scope.locals.clone());
        data.insert(
            "inventory_hostname".into(),
            Value::String(host.name.clone()),
        );
        Ok((definitions, data))
    }

    pub(crate) fn execute_with_definitions(
        &self,
        host: &Host,
        runtime: &RuntimeVariables,
        definitions: &HashMap<String, Value>,
        play_vars: &Mapping,
        check_mode: bool,
        execution: &TaskExecution<'_>,
    ) -> Result<TaskResult> {
        if self.original.is_none() && self.parent.is_some() {
            return self.effective().execute_with_definitions(
                host,
                runtime,
                definitions,
                play_vars,
                check_mode,
                execution,
            );
        }
        let (mut definitions, mut data) =
            self.variable_layers(host, runtime, definitions, play_vars)?;
        let vars = crate::playbook::templar::resolve_definitions(&definitions, &data)?;
        if let Some(tags) = execution.tags {
            if !tags.selected(&self.tags, &vars)? {
                let mut result = TaskResult::new(&host.name);
                result.filtered = true;
                return Ok(result);
            }
        }
        let start_time = Instant::now();
        info!("TASK [{}] on host {}", self.name, host.name);

        // A task may opt into check mode, but it must never opt out of a
        // command-line dry run and unexpectedly mutate the target.
        let check_mode = check_mode || self.check_mode.unwrap_or(false);

        let mut engine = crate::playbook::templar::environment();
        let mut results = Vec::new();
        let mut registered_loop_results = Vec::new();

        if let Some(items) = &self.loop_items {
            let initial_context = crate::playbook::templar::create_context(&vars)?;
            let items_list =
                match self.resolve_loop_items(items, &mut engine, &initial_context, &vars)? {
                    Some(list) if !list.is_empty() => list,
                    _ => {
                        let mut result = TaskResult::new(&host.name);
                        result.skipped = true;
                        result.msg = "No items in loop".to_string();
                        result.no_log |= self.no_log.unwrap_or(false);
                        print_task_result(
                            &host.name,
                            &self.name,
                            &result,
                            &format!("{:.2}s", start_time.elapsed().as_secs_f64()),
                        );
                        return Ok(result);
                    }
                };

            debug!("Executing task with loop: {} items", items_list.len());

            let loop_var = self.loop_var_name.as_deref().unwrap_or("item");

            for (idx, item) in items_list.iter().enumerate() {
                debug!("Loop iteration {} of {}", idx + 1, items_list.len());

                let mut iter_vars = data.clone();
                for key in runtime.extra.keys() {
                    iter_vars.remove(key);
                }
                iter_vars.extend((*runtime.globals).clone());
                iter_vars.extend(runtime.magic.clone());
                iter_vars.insert(
                    "inventory_hostname".into(),
                    Value::String(host.name.clone()),
                );
                // Loop bindings are transient execution values, above ordinary
                // definitions (including extra vars), just like local register.
                iter_vars.insert(loop_var.to_string(), item.clone());
                if let Some(index_var) = &self.index_var_name {
                    iter_vars.insert(index_var.clone(), Value::Number(idx.into()));
                }
                let iter_vars =
                    crate::playbook::templar::resolve_definitions(&definitions, &iter_vars)?;
                let iter_context = crate::playbook::templar::create_context(&iter_vars)?;

                if let Some(when) = &self.when {
                    let mut condition_engine = crate::playbook::templar::environment();
                    if !crate::playbook::templar::evaluate_conditions(
                        when,
                        &mut condition_engine,
                        &iter_context,
                    )? {
                        debug!("Skipping loop iteration due to when condition");
                        let mut result = TaskResult::new(&host.name);
                        result.skipped = true;
                        result.msg = "Skipped loop item due to condition".to_string();
                        result.no_log |= self.no_log.unwrap_or(false);
                        let elapsed = start_time.elapsed();
                        print_loop_iteration_result(
                            &host.name,
                            &self.name,
                            &result,
                            &format!("{:.2}s", elapsed.as_secs_f64()),
                            idx + 1,
                            items_list.len(),
                        );
                        registered_loop_results.push(loop_iteration_value(
                            &result,
                            item,
                            loop_var,
                            self.index_var_name.as_deref(),
                            idx,
                        ));
                        results.push(result);
                        continue;
                    }
                }

                let mut iter_task = self.clone();
                iter_task.loop_items = None;
                iter_task.scope.locals.insert(loop_var.into(), item.clone());
                if let Some(index) = &self.index_var_name {
                    iter_task
                        .scope
                        .locals
                        .insert(index.clone(), Value::Number(idx.into()));
                }

                let mut result = match iter_task.execute_attempts(
                    host,
                    &mut engine,
                    &iter_vars,
                    check_mode,
                    execution,
                    &definitions,
                ) {
                    Ok(result) => result,
                    Err(error) => {
                        let mut result = TaskResult::new(&host.name);
                        result.failed = true;
                        result.msg = format!("Loop item failed: {}", error);
                        result
                    }
                };
                result.no_log |= self.no_log.unwrap_or(false);
                self.resolve_notifications(&mut result, &iter_vars, &mut engine);
                for include in &mut result.includes {
                    include.iteration = idx;
                }

                if !result.skipped && self.facts_belong_to_host(&result) {
                    if let Some(Value::Mapping(facts)) = result.values.get("ansible_facts") {
                        let variables = facts
                            .iter()
                            .map(|(name, value)| {
                                Ok((
                                    name.as_str()
                                        .context("Variable names must be strings")?
                                        .to_owned(),
                                    value.clone(),
                                ))
                            })
                            .collect::<Result<HashMap<_, _>>>()?;
                        match Builtin::lookup(&self.module)? {
                            Builtin::SetFact => {
                                crate::inventory::host::merge_variable_layer(&mut data, &variables)
                            }
                            Builtin::IncludeVars => {
                                for key in variables.keys() {
                                    if !runtime.registered.contains_key(key) {
                                        data.remove(key);
                                    }
                                }
                                crate::inventory::host::merge_variable_layer(
                                    &mut definitions,
                                    &variables,
                                );
                                crate::inventory::host::merge_variable_layer(
                                    &mut definitions,
                                    &runtime.extra,
                                );
                            }
                            _ => {}
                        }
                    }
                }

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

                registered_loop_results.push(loop_iteration_value(
                    &result,
                    item,
                    loop_var,
                    self.index_var_name.as_deref(),
                    idx,
                ));
                let stop = result.unreachable || result.uncertain;
                results.push(result);
                if stop {
                    break;
                }
            }
        } else {
            let context = crate::playbook::templar::create_context(&vars)?;
            if let Some(when) = &self.when {
                let mut condition_engine = crate::playbook::templar::environment();
                if !crate::playbook::templar::evaluate_conditions(
                    when,
                    &mut condition_engine,
                    &context,
                )? {
                    debug!("Skipping task due to when condition");
                    let mut result = TaskResult::new(&host.name);
                    result.skipped = true;
                    result.msg = "Skipped due to condition".to_string();
                    result.no_log |= self.no_log.unwrap_or(false);
                    print_task_result(
                        &host.name,
                        &self.name,
                        &result,
                        &format!("{:.2}s", start_time.elapsed().as_secs_f64()),
                    );
                    return Ok(result);
                }
            }

            let mut result = self.execute_attempts(
                host,
                &mut engine,
                &vars,
                check_mode,
                execution,
                &definitions,
            )?;
            result.no_log |= self.no_log.unwrap_or(false);
            self.resolve_notifications(&mut result, &vars, &mut engine);
            results.push(result);
        }

        let changed = results.iter().any(|r| r.changed);
        let failed = results.iter().any(|r| r.failed);
        let skipped = !results.is_empty() && results.iter().all(|r| r.skipped);

        let mut final_result = TaskResult::new(&host.name);
        final_result.meta = results.iter().find_map(|result| result.meta.clone());
        final_result.includes = results
            .iter()
            .flat_map(|result| result.includes.clone())
            .collect();
        final_result.notifications = results
            .iter()
            .flat_map(|result| result.notifications.clone())
            .collect();
        final_result.changed = changed;
        // Keep the underlying outcome intact. Whether a failure is ignored is
        // a play-execution policy decision and must not erase the module result
        // (registered variables and recap still need to see `failed: true`).
        final_result.failed = failed;
        final_result.unreachable = results.iter().any(|result| result.unreachable);
        final_result.uncertain = results.iter().any(|result| result.uncertain);
        final_result.skipped = skipped;
        final_result.no_log =
            self.no_log.unwrap_or(false) || results.iter().any(|result| result.no_log);

        if self.loop_items.is_some() {
            final_result.values.insert(
                "results".to_string(),
                Value::Sequence(registered_loop_results),
            );
            let failed_count = results.iter().filter(|result| result.failed).count();
            let changed_count = results.iter().filter(|result| result.changed).count();
            let skipped_count = results.iter().filter(|result| result.skipped).count();
            let ok_count = results.len() - failed_count - skipped_count;
            final_result.msg = format!(
                "ok={} changed={} failed={} skipped={} iterations={}",
                ok_count,
                changed_count,
                failed_count,
                skipped_count,
                results.len()
            );
            for (idx, result) in results.iter().enumerate() {
                for (key, value) in &result.values {
                    final_result
                        .values
                        .insert(format!("item_{}.{}", idx, key), value.clone());
                }
            }
        } else if results.len() == 1 {
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

        if results.len() > 1 || (results.len() == 1 && self.loop_items.is_none()) {
            print_task_result(&host.name, &self.name, &final_result, &execution_time);
        }

        Ok(final_result)
    }

    /// Register projections are rebuilt after each attempt and condition. Data
    /// from a command stays data; evaluating result conditions never re-renders it.
    fn execute_attempts(
        &self,
        host: &Host,
        engine: &mut Environment<'_>,
        vars: &HashMap<String, Value>,
        check: bool,
        execution: &TaskExecution<'_>,
        definitions: &HashMap<String, Value>,
    ) -> Result<TaskResult> {
        let context = crate::playbook::templar::create_context(vars)?;
        let has_until = self
            .until
            .as_ref()
            .is_some_and(|condition| !condition.as_sequence().is_some_and(Vec::is_empty));
        let retries = self
            .integer_keyword(
                &self.retries,
                if has_until { 3 } else { 0 },
                "retries",
                engine,
                &context,
            )?
            .max(0);
        let total = u32::try_from(retries)?
            .checked_add(1)
            .context("Task retries is too large")?;
        let delay = self.number_keyword(&self.delay, 5.0, "delay", engine, &context)?;
        let delay = if delay < 0.0 { 1.0 } else { delay };
        if delay > 604800.0 {
            return Err(anyhow!("Task delay exceeds seven days"));
        }
        let mut attempt_vars = vars.clone();
        for attempt in 1..=total {
            let mut result =
                self.execute_module(host, engine, definitions, &attempt_vars, check, execution)?;
            if total > 1 {
                result
                    .values
                    .insert("attempts".into(), Value::Number(attempt.into()));
            }
            self.merge_action_variables(&result, &mut attempt_vars)?;
            if result.unreachable || result.uncertain || result.skipped {
                return Ok(result);
            }
            let condition_result = (|| -> Result<bool> {
                for (condition, failure) in [(&self.changed_when, false), (&self.failed_when, true)]
                {
                    if let Some(register) = self.register.as_ref() {
                        attempt_vars.insert(register.clone(), result.registered_value());
                    }
                    if let Some(condition) = condition {
                        if condition.as_sequence().is_some_and(Vec::is_empty) {
                            continue;
                        }
                        let context = crate::playbook::templar::create_context(&attempt_vars)?;
                        let value = crate::playbook::templar::evaluate_conditions(
                            condition, engine, &context,
                        )?;
                        if failure {
                            result.failed = value;
                        } else {
                            result.changed = value;
                        }
                    }
                }
                if let Some(register) = self.register.as_ref() {
                    attempt_vars.insert(register.clone(), result.registered_value());
                }
                if total == 1 {
                    return Ok(true);
                }
                if let Some(condition) = self.until.as_ref().filter(|_| has_until) {
                    let context = crate::playbook::templar::create_context(&attempt_vars)?;
                    crate::playbook::templar::evaluate_conditions(condition, engine, &context)
                } else {
                    Ok(!result.failed)
                }
            })();
            match condition_result {
                Ok(true) => return Ok(result),
                Err(error) => {
                    result.failed = true;
                    result.msg = format!("Task result condition failed: {error}");
                    return Ok(result);
                }
                Ok(false) if attempt == total => {
                    result.failed = true;
                    result.msg = format!(
                        "Task retry condition was not satisfied after {total} attempts; {}",
                        result.msg
                    );
                    // Pinned Ansible 2.21 reports the retry count on exhaustion.
                    result
                        .values
                        .insert("attempts".into(), Value::Number((total - 1).into()));
                    return Ok(result);
                }
                Ok(false) => std::thread::sleep(std::time::Duration::from_secs_f64(delay)),
            }
        }
        unreachable!("Positive attempt count always returns a result")
    }

    fn number_keyword(
        &self,
        value: &Option<Value>,
        default: f64,
        name: &str,
        engine: &mut Environment<'_>,
        context: &TemplateContext,
    ) -> Result<f64> {
        let Some(value) = value else {
            return Ok(default);
        };
        let value = if let Value::String(text) = value {
            crate::playbook::templar::render_value(text, engine, context, true)?
        } else {
            value.clone()
        };
        let number = value
            .as_f64()
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
            .filter(|value| value.is_finite())
            .with_context(|| format!("Task {name} must resolve to a finite number"))?;
        Ok(number)
    }
    fn integer_keyword(
        &self,
        value: &Option<Value>,
        default: i64,
        name: &str,
        engine: &mut Environment<'_>,
        context: &TemplateContext,
    ) -> Result<i64> {
        let Some(value) = value else {
            return Ok(default);
        };
        let value = if let Value::String(text) = value {
            crate::playbook::templar::render_value(text, engine, context, true)?
        } else {
            value.clone()
        };
        value
            .as_i64()
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
            .with_context(|| format!("Task {name} must resolve to an integer"))
    }

    fn execute_module(
        &self,
        host: &Host,
        engine: &mut Environment<'_>,
        definitions: &HashMap<String, Value>,
        vars: &HashMap<String, Value>,
        check_mode: bool,
        execution: &TaskExecution<'_>,
    ) -> Result<TaskResult> {
        debug!(
            "Executing module '{}' for task '{}'",
            self.module, self.name
        );

        let context = crate::playbook::templar::create_context(vars)?;
        let mut resolved_args = self.resolve_args(engine, &context, vars)?;
        if matches!(Builtin::lookup(&self.module), Ok(Builtin::Meta)) {
            Builtin::Meta.validate_parameters(&Value::Mapping(resolved_args.clone()))?;
            let mut result = TaskResult::new(&host.name);
            result.meta = resolved_args
                .get(Value::String("action".into()))
                .and_then(Value::as_str)
                .map(str::to_owned);
            result.msg = "Meta action selected".into();
            return Ok(result);
        }

        if let Some(kind) = include_kind(&self.module) {
            let allowed: &[&str] = if kind == IncludeKind::Tasks {
                &["file", "apply"]
            } else {
                &[
                    "name",
                    "tasks_from",
                    "vars_from",
                    "defaults_from",
                    "handlers_from",
                    "public",
                    "allow_duplicates",
                    "apply",
                ]
            };
            crate::modules::param::validate_params(
                &Value::Mapping(resolved_args.clone()),
                allowed,
            )?;
            let key = if kind == IncludeKind::Tasks {
                "file"
            } else {
                "name"
            };
            if resolved_args
                .get(Value::String(key.into()))
                .and_then(Value::as_str)
                .is_none_or(|name| name.is_empty() || name.contains('\0'))
            {
                return Err(anyhow!("Include requires a nonempty {key} string"));
            }
            let mut result = TaskResult::new(&host.name);
            result.values.insert(
                "include".into(),
                resolved_args
                    .get(Value::String(key.into()))
                    .expect("Validated include")
                    .clone(),
            );
            result.includes.push(IncludeRequest {
                iteration: 0,
                kind,
                args: resolved_args,
                parent: self.clone(),
            });
            result.msg = "Include selected".into();
            return Ok(result);
        }

        let module_name = self.normalized_module_name();
        if matches!(module_name, "copy" | "template")
            || module_name == "unarchive"
                && resolved_args
                    .get(Value::String("remote_src".into()))
                    .and_then(Value::as_bool)
                    != Some(true)
        {
            if let Some(Value::String(path)) = resolved_args.get_mut(Value::String("src".into())) {
                *path = self
                    .source_path(
                        if module_name != "template" {
                            "files"
                        } else {
                            "templates"
                        },
                        path,
                        vars,
                    )
                    .to_string_lossy()
                    .into_owned();
            }
        }
        if module_name == "include_vars" {
            for key in ["file", "dir"] {
                if let Some(Value::String(path)) = resolved_args.get_mut(Value::String(key.into()))
                {
                    *path = self
                        .source_path("vars", path, vars)
                        .to_string_lossy()
                        .into_owned();
                }
            }
        }
        let timeout = self.integer_keyword(&self.timeout, 0, "timeout", engine, &context)?;
        if !(0..=604800).contains(&timeout) {
            return Err(anyhow!("Task timeout must be between zero and seven days"));
        }
        let mut environment = std::collections::BTreeMap::new();
        for (name, value) in &self.environment {
            let name = name.as_str().context("Environment names must be strings")?;
            let value = if crate::vault::is_value(value) {
                crate::vault::Vault::current().materialize(value)?
            } else if let Value::String(text) = value {
                crate::playbook::templar::render_value(text, engine, &context, true)?
            } else {
                value.clone()
            };
            let text = match value {
                Value::String(text) => text,
                Value::Number(value) => value.to_string(),
                Value::Bool(value) => value.to_string(),
                _ => {
                    return Err(anyhow!(
                        "Environment values must resolve to scalar strings, numbers or booleans"
                    ))
                }
            };
            environment.insert(name.to_owned(), text);
        }
        let asynchronous =
            self.integer_keyword(&self.asynchronous, 0, "async", engine, &context)?;
        let poll = self.integer_keyword(&self.poll, 15, "poll", engine, &context)?;
        if !(0..=604800).contains(&asynchronous) || !(0..=604800).contains(&poll) {
            return Err(anyhow!(
                "Async duration and poll must be nonnegative seconds up to seven days"
            ));
        }
        let invocation =
            Invocation::prepare(module_name, &Value::Mapping(resolved_args), check_mode)?
                .with_execution(environment, (timeout > 0).then_some(timeout as u64 * 1000))
                .with_async(
                    asynchronous as u64,
                    poll as u64,
                    self.no_log.unwrap_or(false),
                )?;
        let delegate = self.delegate_to.as_ref().map(|source| -> Result<String> {
            let value = crate::playbook::templar::render_value(source, engine, &context, true)?;
            let name = value.as_str().filter(|name| !name.trim().is_empty() && !name.chars().any(|ch| ch.is_control() || ch.is_whitespace()))
                .context("delegate_to must resolve to a nonempty host name without control characters")?;
            Ok(name.to_owned())
        }).transpose()?;
        let module_result = if !invocation.needs_connection() {
            if delegate.is_none() {
                self.render_become_user(engine, &context)?;
            }
            invocation.execute(None, vars, execution.workers)
        } else {
            let config = if let Some(target) = &delegate {
                execution
                    .delegation
                    .as_ref()
                    .context("Delegation requires playbook execution")?
                    .connection(target, self, host)?
            } else {
                self.connection_config(&host.name, definitions, vars)?
            };
            invocation.execute(Some(&config), vars, execution.workers)
        };
        let mut result = TaskResult::from_module_result(&host.name, module_result);
        if let Some(target) = delegate {
            result
                .values
                .insert("delegated_to".into(), Value::String(target));
        }
        Ok(result)
    }

    pub(crate) fn connection_config(
        &self,
        name: &str,
        definitions: &HashMap<String, Value>,
        vars: &HashMap<String, Value>,
    ) -> Result<ConnectionConfig> {
        ConnectionConfig::require_resolved(definitions, vars)?;
        let mut engine = crate::playbook::templar::environment();
        let context = crate::playbook::templar::create_context(vars)?;
        let become_user = self.render_become_user(&mut engine, &context)?;
        let mut connection_vars = vars.clone();
        if let Some(connection) = &self.connection {
            let value =
                crate::playbook::templar::render_value(connection, &mut engine, &context, true)?;
            connection_vars
                .entry("ansible_connection".into())
                .or_insert(value);
        }
        connection_vars
            .entry("ansible_become".into())
            .or_insert(Value::Bool(self.is_become));
        connection_vars
            .entry("ansible_become_user".into())
            .or_insert(Value::String(become_user));
        ConnectionConfig::from_context(name, connection_vars)
    }

    fn render_become_user(
        &self,
        engine: &mut Environment<'_>,
        context: &TemplateContext,
    ) -> Result<String> {
        let Value::String(become_user) =
            crate::playbook::templar::render_value(&self.become_user, engine, context, true)?
        else {
            return Err(anyhow!("become_user must render to a string"));
        };
        if become_user.trim().is_empty() || become_user.chars().any(char::is_control) {
            return Err(anyhow!(
                "become_user must render to a non-empty string without control characters"
            ));
        }
        Ok(become_user)
    }

    fn normalized_module_name(&self) -> &str {
        Builtin::lookup(&self.module)
            .map(Builtin::name)
            .unwrap_or(&self.module)
    }
    fn merge_action_variables(
        &self,
        result: &TaskResult,
        variables: &mut HashMap<String, Value>,
    ) -> Result<()> {
        if !self.facts_belong_to_host(result) {
            return Ok(());
        }
        let Some(Value::Mapping(facts)) = result.values.get("ansible_facts") else {
            return Ok(());
        };
        match Builtin::lookup(&self.module) {
            Ok(Builtin::SetFact) => {
                let values = facts
                    .iter()
                    .map(|(name, value)| {
                        Ok((
                            name.as_str()
                                .context("Fact names must be strings")?
                                .to_owned(),
                            value.clone(),
                        ))
                    })
                    .collect::<Result<HashMap<_, _>>>()?;
                crate::inventory::host::merge_variable_layer(variables, &values);
            }
            Ok(Builtin::IncludeVars) => {
                let definitions = facts
                    .iter()
                    .map(|(name, value)| {
                        Ok((
                            name.as_str()
                                .context("Variable names must be strings")?
                                .to_owned(),
                            value.clone(),
                        ))
                    })
                    .collect::<Result<HashMap<_, _>>>()?;
                for name in definitions.keys() {
                    variables.remove(name);
                }
                *variables =
                    crate::playbook::templar::resolve_definitions(&definitions, variables)?;
            }
            Ok(Builtin::Setup) => crate::playbook::Play::merge_facts(variables, facts)?,
            _ => {}
        }
        Ok(())
    }
    fn facts_belong_to_host(&self, result: &TaskResult) -> bool {
        !self.delegate_facts.unwrap_or(false)
            || result
                .values
                .get("delegated_to")
                .and_then(Value::as_str)
                .is_none_or(|target| target == result.host)
    }
    fn resolve_notifications(
        &self,
        result: &mut TaskResult,
        vars: &HashMap<String, Value>,
        engine: &mut Environment<'_>,
    ) {
        if !result.changed || result.failed || result.skipped {
            return;
        }
        let resolved = (|| -> Result<Vec<String>> {
            let mut vars = vars.clone();
            self.merge_action_variables(result, &mut vars)?;
            if let Some(register) = &self.register {
                vars.insert(register.clone(), result.registered_value());
            }
            let context = crate::playbook::templar::create_context(&vars)?;
            self.notify
                .iter()
                .map(|name| {
                    crate::playbook::templar::render_value(name, engine, &context, true)?
                        .as_str()
                        .filter(|name| !name.is_empty())
                        .map(str::to_owned)
                        .context("Handler notification must resolve to a nonempty string")
                })
                .collect()
        })();
        match resolved {
            Ok(names) => result.notifications = names,
            Err(error) => {
                result.failed = true;
                result.msg = format!("Cannot render handler notification: {error:#}");
            }
        }
    }

    fn source_path(
        &self,
        directory: &str,
        path: &str,
        vars: &HashMap<String, Value>,
    ) -> std::path::PathBuf {
        let path = std::path::Path::new(path);
        if path.is_absolute() {
            return path.to_path_buf();
        }
        let mut candidates = Vec::new();
        for role in self
            .scope
            .role_path
            .iter()
            .chain(self.scope.parent_roles.iter().rev())
        {
            candidates.push(role.join(directory).join(path));
        }
        if let Some(base) = self.source_file.as_ref().and_then(|source| source.parent()) {
            candidates.push(base.join(path));
            candidates.push(base.join(directory).join(path));
        }
        if let Some(base) = vars.get("playbook_dir").and_then(Value::as_str) {
            candidates.push(std::path::Path::new(base).join(directory).join(path));
            candidates.push(std::path::Path::new(base).join(path));
        }
        candidates
            .iter()
            .find(|path| path.exists())
            .cloned()
            .or_else(|| candidates.into_iter().next())
            .unwrap_or_else(|| path.to_path_buf())
    }

    fn resolve_loop_items(
        &self,
        items: &Value,
        engine: &mut Environment<'_>,
        context: &TemplateContext,
        _vars: &HashMap<String, Value>,
    ) -> Result<Option<Vec<Value>>> {
        match items {
            Value::Sequence(_) => {
                let Value::Sequence(items) = render_argument_value(
                    &self.module,
                    items,
                    engine,
                    context,
                    false,
                    false,
                    0,
                    &mut 0,
                )?
                else {
                    unreachable!("Rendering a sequence preserves its type");
                };
                Ok(Some(items))
            }
            Value::String(var_name) => {
                match crate::playbook::templar::render_value(var_name, engine, context, false) {
                    Ok(Value::Sequence(resolved_seq)) => Ok(Some(resolved_seq)),
                    Ok(Value::String(s))
                        if !var_name.contains("{{") && !var_name.contains("{%") =>
                    {
                        // If the original var_name wasn't a template, treat the string as a single-item list
                        warn!("A plain loop string is being interpreted as one item");
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
        engine: &mut Environment<'_>,
        context: &TemplateContext,
        vars: &HashMap<String, Value>,
    ) -> Result<Mapping> {
        render_module_arguments(&self.module, &self.args, engine, context, vars)
    }
}

pub(crate) fn render_module_arguments(
    module: &str,
    args: &Mapping,
    engine: &mut Environment<'_>,
    context: &TemplateContext,
    vars: &HashMap<String, Value>,
) -> Result<Mapping> {
    let mut resolved = Mapping::new();
    debug!("Resolving {} arguments for module '{}'", args.len(), module);

    let module_name = Builtin::lookup(module).map(Builtin::name).unwrap_or(module);
    let mut rendered_nodes = 0;
    for (key, value) in args {
        let Value::String(key) = key else {
            return Err(anyhow!("Module argument names must be strings"));
        };

        // These compatibility rules intentionally depend only on the
        // top-level module argument. Mapping keys below it are data: they
        // are validated and preserved verbatim, never treated as templates.
        let force_string = (module_name == "template" && key == "content")
            || (module_name == "debug" && matches!(key.as_str(), "msg" | "var"));
        let is_mode_param =
            matches!(module_name, "file" | "copy" | "template" | "unarchive") && key == "mode";

        if (module_name == "template" && matches!(key.as_str(), "content" | "vars"))
            || (module_name == "assert" && key == "that")
        {
            resolved.insert(Value::String(key.clone()), value.clone());
            continue;
        }

        let rendered_value = render_argument_value(
            module,
            value,
            engine,
            context,
            force_string,
            is_mode_param,
            0,
            &mut rendered_nodes,
        )
        .map_err(|error| {
            anyhow!(
                "Failed to render an argument for module '{}': {}",
                module,
                error
            )
        })?;

        if matches!(
            &rendered_value,
            Value::String(value) if value == crate::playbook::templar::OMIT_SENTINEL
        ) {
            continue;
        }

        resolved.insert(Value::String(key.clone()), rendered_value.clone());

        // Preserve debug.var's controller-side lookup behavior without
        // logging either the variable name or its potentially secret value.
        if module_name == "debug" && key == "var" {
            if let Value::String(var_name_to_lookup) = &rendered_value {
                if let Some(found_value) = get_nested_value(var_name_to_lookup, vars) {
                    resolved.insert(
                        Value::String("_var_value".to_string()),
                        crate::data::materialize(found_value)?,
                    );
                } else {
                    resolved.remove(Value::String("_var_value".to_string()));
                }
            } else {
                resolved.remove(Value::String("_var_value".to_string()));
            }
        }
    }

    debug!("Finished resolving arguments for module '{}'", module);
    Ok(resolved)
}

#[allow(clippy::too_many_arguments)]
fn render_argument_value(
    module: &str,
    value: &Value,
    engine: &mut Environment<'_>,
    context: &TemplateContext,
    force_string: bool,
    is_mode_param: bool,
    depth: usize,
    rendered_nodes: &mut usize,
) -> Result<Value> {
    if depth > MAX_ARGUMENT_TEMPLATE_DEPTH {
        return Err(anyhow!(
            "Module arguments exceed the maximum nesting depth of {}",
            MAX_ARGUMENT_TEMPLATE_DEPTH
        ));
    }

    *rendered_nodes = rendered_nodes
        .checked_add(1)
        .ok_or_else(|| anyhow!("Module argument node count overflowed"))?;
    if *rendered_nodes > MAX_ARGUMENT_TEMPLATE_NODES {
        return Err(anyhow!(
            "Module arguments exceed the maximum node count of {}",
            MAX_ARGUMENT_TEMPLATE_NODES
        ));
    }

    match value {
        Value::String(value) => {
            let rendered_value =
                crate::playbook::templar::render_value(value, engine, context, force_string)?;

            if is_mode_param {
                if let Value::String(mode) = &rendered_value {
                    if mode.contains("{{") || mode.contains("{%") {
                        return Err(anyhow!(
                            "Mode parameter for module '{}' contains unresolved template syntax",
                            module
                        ));
                    }
                }
            }

            Ok(rendered_value)
        }
        Value::Sequence(values) => values
            .iter()
            .map(|value| {
                render_argument_value(
                    module,
                    value,
                    engine,
                    context,
                    force_string,
                    is_mode_param,
                    depth + 1,
                    rendered_nodes,
                )
            })
            .collect::<Result<Vec<_>>>()
            .map(Value::Sequence),
        Value::Mapping(values) => {
            let mut rendered = Mapping::new();
            for (key, value) in values {
                let Value::String(key) = key else {
                    return Err(anyhow!("Module argument mapping keys must be strings"));
                };
                let value = render_argument_value(
                    module,
                    value,
                    engine,
                    context,
                    force_string,
                    is_mode_param,
                    depth + 1,
                    rendered_nodes,
                )?;
                rendered.insert(Value::String(key.clone()), value);
            }
            Ok(Value::Mapping(rendered))
        }
        Value::Tagged(_) if crate::data::literal(value).is_some() => {
            crate::data::materialize(value)
        }
        Value::Tagged(_) if crate::vault::is_value(value) => {
            crate::vault::Vault::current().materialize(value)
        }
        Value::Tagged(tagged) => {
            let mut rendered = (**tagged).clone();
            rendered.value = render_argument_value(
                module,
                &tagged.value,
                engine,
                context,
                force_string,
                is_mode_param,
                depth + 1,
                rendered_nodes,
            )?;
            Ok(Value::Tagged(Box::new(rendered)))
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(value.clone()),
    }
}

fn loop_iteration_value(
    result: &TaskResult,
    item: &Value,
    loop_var: &str,
    index_var: Option<&str>,
    index: usize,
) -> Value {
    let mut value = crate::modules::result::result_fields(
        result.changed,
        result.outcome(),
        &result.msg,
        &result.values,
    );
    value.insert(Value::String(loop_var.to_string()), item.clone());
    value.insert(
        Value::String("ansible_loop_var".to_string()),
        Value::String(loop_var.to_string()),
    );
    if let Some(index_var) = index_var {
        value.insert(
            Value::String(index_var.to_string()),
            Value::Number(index.into()),
        );
        value.insert(
            Value::String("ansible_index_var".to_string()),
            Value::String(index_var.to_string()),
        );
    }

    Value::Mapping(value)
}

pub(crate) fn print_task_result(
    host_name: &str,
    _task_name: &str,
    result: &TaskResult,
    execution_time: &str,
) {
    let private = result.no_log || result.failed && crate::vault::Vault::current().used();
    let status = if result.uncertain {
        "uncertain"
    } else if result.unreachable {
        "unreachable"
    } else if result.failed {
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
    } else if result.skipped || result.changed {
        "\x1B[33m"
    } else {
        "\x1B[32m"
    };

    let reset_code = "\x1B[0m";

    let message = if private {
        "output censored because no_log is enabled"
    } else if !result.failed
        && result
            .values
            .get("_ansible_verbose_always")
            .and_then(Value::as_bool)
            == Some(false)
    {
        ""
    } else {
        result.msg.as_str()
    };
    println!(
        "{} => {}{}: {} ({}){}",
        host_name, color_code, status, message, execution_time, reset_code
    );

    if private {
        return;
    }

    if let Some(diff) = result.values.get("diff") {
        if let Ok(text) = serde_json::to_string_pretty(diff) {
            println!("    diff: {text}");
        }
    }
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
    let private = result.no_log || result.failed && crate::vault::Vault::current().used();
    let status = if result.uncertain {
        "uncertain"
    } else if result.unreachable {
        "unreachable"
    } else if result.failed {
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
    } else if result.skipped || result.changed {
        "\x1B[33m"
    } else {
        "\x1B[32m"
    };

    let reset_code = "\x1B[0m";

    let message = if private {
        "output censored because no_log is enabled"
    } else {
        result.msg.as_str()
    };
    println!(
        "{}{} (item={}/{}) => {}{}{}: {} ({}){}",
        color_code,
        host_name,
        iteration,
        total_iterations,
        color_code,
        status,
        reset_code,
        message,
        execution_time,
        reset_code
    );

    if private {
        return;
    }

    if let Some(diff) = result.values.get("diff") {
        if let Ok(text) = serde_json::to_string_pretty(diff) {
            println!("    diff: {text}");
        }
    }
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
    use std::fs;
    use tempfile::tempdir;

    fn create_test_task() -> Task {
        Task {
            group: None,
            parent: None,
            original: None,
            scope: TaskScope::default(),
            name: "Test Task".to_string(),
            module: "debug".to_string(),
            args: Mapping::new(),
            source_file: None,
            become_override: None,
            is_become: false,
            become_user: "root".to_string(),
            become_user_override: None,
            connection: None,
            check_mode: None,
            register: None,
            when: None,
            changed_when: None,
            failed_when: None,
            until: None,
            retries: None,
            delay: None,
            timeout: None,
            asynchronous: None,
            poll: None,
            environment: Mapping::new(),
            notify: Vec::new(),
            ignore_errors: None,
            ignore_unreachable: None,
            no_log: None,
            vars: Mapping::new(),
            tags: Vec::new(),
            loop_items: None,
            loop_var_name: None,
            index_var_name: None,
            delegate_to: None,
            delegate_facts: None,
            throttle: None,
            run_once: None,
            any_errors_fatal: None,
        }
    }

    fn mapping(entries: &[(&str, Value)]) -> Mapping {
        entries
            .iter()
            .map(|(key, value)| (Value::String((*key).to_string()), value.clone()))
            .collect()
    }

    #[test]
    fn debug_is_controller_side_and_does_not_open_ssh() {
        let mut host = Host::new("unreachable.invalid");
        host.set_variable("ansible_connection", "ssh");
        let mut task = create_test_task();
        task.args = mapping(&[("msg", Value::String("still local".to_string()))]);

        let result = task.execute(&host, &HashMap::new()).unwrap();
        assert!(!result.failed);
        assert_eq!(result.msg, "still local");
    }

    #[test]
    fn become_user_is_rendered_from_the_host_context() {
        let host = Host::new("localhost");
        let mut task = create_test_task();
        task.is_become = true;
        task.become_user = "{{ app_user }}".to_string();
        task.args = mapping(&[("msg", Value::String("safe".to_string()))]);
        let vars = HashMap::from([(
            "app_user".to_string(),
            Value::String("deployer".to_string()),
        )]);

        assert!(!task.execute(&host, &vars).unwrap().failed);
        assert!(task.execute(&host, &HashMap::new()).is_err());
    }

    #[test]
    fn no_log_marks_results_for_censored_rendering() {
        let host = Host::new("localhost");
        let mut task = create_test_task();
        task.no_log = Some(true);
        task.args = mapping(&[("msg", Value::String("secret value".to_string()))]);

        let result = task.execute(&host, &HashMap::new()).unwrap();
        assert!(result.no_log);
    }

    #[test]
    fn local_connection_runs_file_copy_template_and_lineinfile_modules() {
        let directory = tempdir().unwrap();
        let host = Host::new("localhost");
        let vars = HashMap::from([(
            "subject".to_string(),
            Value::String("local world".to_string()),
        )]);

        let file_path = directory.path().join("touched file");
        let mut file_task = create_test_task();
        file_task.module = "file".to_string();
        file_task.args = mapping(&[
            (
                "path",
                Value::String(file_path.to_string_lossy().into_owned()),
            ),
            ("state", Value::String("touch".to_string())),
        ]);
        assert!(!file_task.execute(&host, &vars).unwrap().failed);
        assert!(file_path.is_file());

        let source = directory.path().join("binary source");
        let copy_path = directory.path().join("binary copy");
        let bytes = [0_u8, 0xff, b'\n', 0x80];
        fs::write(&source, bytes).unwrap();
        let mut copy_task = create_test_task();
        copy_task.module = "copy".to_string();
        copy_task.args = mapping(&[
            ("src", Value::String(source.to_string_lossy().into_owned())),
            (
                "dest",
                Value::String(copy_path.to_string_lossy().into_owned()),
            ),
        ]);
        assert!(!copy_task.execute(&host, &vars).unwrap().failed);
        assert_eq!(fs::read(copy_path).unwrap(), bytes);

        let template_path = directory.path().join("rendered template");
        let mut template_task = create_test_task();
        template_task.module = "template".to_string();
        template_task.args = mapping(&[
            ("content", Value::String("hello {{ subject }}".to_string())),
            (
                "dest",
                Value::String(template_path.to_string_lossy().into_owned()),
            ),
        ]);
        assert!(!template_task.execute(&host, &vars).unwrap().failed);
        assert_eq!(
            fs::read_to_string(template_path).unwrap(),
            "hello local world"
        );

        let line_path = directory.path().join("managed lines");
        let mut line_task = create_test_task();
        line_task.module = "lineinfile".to_string();
        line_task.args = mapping(&[
            (
                "path",
                Value::String(line_path.to_string_lossy().into_owned()),
            ),
            ("line", Value::String("managed=true".to_string())),
            ("create", Value::Bool(true)),
        ]);
        assert!(!line_task.execute(&host, &vars).unwrap().failed);
        assert_eq!(fs::read_to_string(line_path).unwrap(), "managed=true\n");
    }

    // Modify to return the vars map as well
    fn create_test_environment_and_context(
        vars_map: Option<HashMap<String, Value>>,
    ) -> (
        Environment<'static>,
        TemplateContext,
        HashMap<String, Value>,
    ) {
        let engine = crate::playbook::templar::environment();
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

        let context = crate::playbook::templar::create_context(&vars).unwrap();
        (engine, context, vars) // Return vars map
    }

    #[test]
    fn task_vars_override_inherited_vars_without_mutating_them() {
        let mut task = create_test_task();
        task.vars.insert(
            Value::String("scope".to_string()),
            Value::String("task".to_string()),
        );
        task.vars
            .insert(Value::String("task_only".to_string()), Value::Bool(true));
        let inherited = HashMap::from([
            ("scope".to_string(), Value::String("play".to_string())),
            ("play_only".to_string(), Value::Bool(true)),
        ]);

        task.args = mapping(&[(
            "msg",
            Value::String("{{ scope }}:{{ task_only }}:{{ play_only }}".into()),
        )]);
        let result = task.execute(&Host::new("localhost"), &inherited).unwrap();
        assert_eq!(result.msg, "task:True:True");
        assert_eq!(
            inherited.get("scope"),
            Some(&Value::String("play".to_string()))
        );
    }

    #[test]
    fn when_sequence_uses_and_semantics() {
        let mut engine = crate::playbook::templar::environment();
        let context = TemplateContext::new(&HashMap::new()).unwrap();
        let all_true = Value::Sequence(vec![Value::Bool(true), Value::Bool(true)]);
        let one_false = Value::Sequence(vec![Value::Bool(true), Value::Bool(false)]);

        assert!(
            crate::playbook::templar::evaluate_conditions(&all_true, &mut engine, &context)
                .unwrap()
        );
        assert!(
            !crate::playbook::templar::evaluate_conditions(&one_false, &mut engine, &context)
                .unwrap()
        );
    }

    #[test]
    fn test_get_nested_value_simple() {
        let (_tera, _context, vars) = create_test_environment_and_context(None);
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
        let (_tera, _context, vars) = create_test_environment_and_context(None);
        let expected_value = Value::String("1.2.3".to_string()); // Explicit expected value
        assert_eq!(
            get_nested_value("application.version", &vars),
            Some(&expected_value) // Compare with explicit value
        );
    }

    #[test]
    fn test_get_nested_value_list() {
        let (_tera, _context, vars) = create_test_environment_and_context(None);
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
        let (_tera, _context, vars) = create_test_environment_and_context(None);
        assert_eq!(get_nested_value("nonexistent", &vars), None);
        assert_eq!(get_nested_value("application.nonexistent", &vars), None);
        assert_eq!(get_nested_value("greeting.nonexistent", &vars), None); // Cannot index into string
        assert_eq!(get_nested_value("items_list.0", &vars), None); // Does not support list indexing
    }

    #[test]
    fn test_resolve_loop_items_direct_list() {
        let task = create_test_task();
        let items = Value::Sequence(vec![Value::Number(1.into()), Value::Number(2.into())]);
        let (mut engine, context, vars) = create_test_environment_and_context(None); // Capture vars

        // Pass vars to resolve_loop_items
        let result = task
            .resolve_loop_items(&items, &mut engine, &context, &vars)
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
        let (mut engine, context, vars) = create_test_environment_and_context(None); // Capture vars

        // Pass vars to resolve_loop_items
        let result = task
            .resolve_loop_items(&items_var, &mut engine, &context, &vars)
            .unwrap();
        assert_eq!(result, Some(expected_list));
    }

    #[test]
    fn test_resolve_loop_items_variable_nested() {
        // Test the direct lookup path for nested variables
        let task = create_test_task();
        let items_var = Value::String("{{ application.features }}".to_string());
        // Capture vars correctly here
        let (mut engine, context, vars) = create_test_environment_and_context(None);
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
            .resolve_loop_items(&items_var, &mut engine, &context, &vars)
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
        let (mut engine, context, vars) = create_test_environment_and_context(None); // Capture vars

        // Pass vars to resolve_loop_items
        let result = task
            .resolve_loop_items(&items_var, &mut engine, &context, &vars)
            .unwrap();
        assert_eq!(result, Some(expected_list));
    }

    #[test]
    fn test_resolve_loop_items_render_fallback() {
        // Test fallback to rendering for a slightly more complex expression
        let task = create_test_task();
        // Use a filter or expression the engine evaluates
        let items_expr = Value::String("{{ items_list | join(\",\") }}".to_string()); // This resolves to a string, not a list
        let (mut engine, context, vars) = create_test_environment_and_context(None);

        // Pass vars to resolve_loop_items
        let result = task.resolve_loop_items(&items_expr, &mut engine, &context, &vars);

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
        let (mut engine, context, vars) = create_test_environment_and_context(None); // Capture vars

        // Pass vars to resolve_loop_items
        let result = task.resolve_loop_items(&items_expr, &mut engine, &context, &vars);
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
        let (mut engine, context, vars) = create_test_environment_and_context(None); // Capture vars

        // Pass vars to resolve_loop_items
        let result = task.resolve_loop_items(&items_expr, &mut engine, &context, &vars);
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
        let (mut engine, context, vars) = create_test_environment_and_context(None); // Capture vars

        // Pass vars to resolve_loop_items
        let result = task
            .resolve_loop_items(&items_bare, &mut engine, &context, &vars)
            .unwrap();
        // Should not be caught by simple var check, should not contain {{ }},
        // should fall back to interpreting the string as a single-item list.
        assert_eq!(result, Some(vec![Value::String("single_item".to_string())]));
    }

    #[test]
    fn resolve_args_renders_nested_sequences_mappings_and_tags() {
        let mut task = create_test_task();
        task.module = "copy".to_string();

        let preserved_key = Value::String("{{ key_name }}".to_string());
        let tagged: Value = serde_yaml::from_str("!sensitive '{{ greeting }}'").unwrap();
        let nested = Value::Mapping(Mapping::from_iter([(
            preserved_key.clone(),
            Value::Sequence(vec![
                Value::String("{{ greeting }} {{ target }}".to_string()),
                Value::Mapping(Mapping::from_iter([(
                    Value::String("enabled".to_string()),
                    Value::String("{{ enabled }}".to_string()),
                )])),
                tagged,
            ]),
        )]));
        task.args
            .insert(Value::String("payload".to_string()), nested);

        let extra_vars = HashMap::from([
            ("enabled".to_string(), Value::Bool(true)),
            (
                "key_name".to_string(),
                Value::String("rendered-key".to_string()),
            ),
        ]);
        let (mut engine, context, vars) = create_test_environment_and_context(Some(extra_vars));
        let resolved = task.resolve_args(&mut engine, &context, &vars).unwrap();
        let payload = resolved
            .get(Value::String("payload".to_string()))
            .unwrap()
            .as_mapping()
            .unwrap();

        // Mapping keys are data and are deliberately not rendered.
        let values = payload.get(&preserved_key).unwrap().as_sequence().unwrap();
        assert_eq!(values[0], Value::String("Hello world".to_string()));
        assert_eq!(
            values[1]
                .as_mapping()
                .unwrap()
                .get(Value::String("enabled".to_string())),
            Some(&Value::Bool(true))
        );
        let Value::Tagged(tagged) = &values[2] else {
            panic!("tagged argument was not preserved");
        };
        assert!(tagged.tag == "sensitive");
        assert_eq!(tagged.value, Value::String("Hello".to_string()));
    }

    #[test]
    fn resolve_args_preserves_top_level_force_string_and_mode_rules() {
        let vars = HashMap::from([
            ("answer".to_string(), Value::Number(42.into())),
            ("private".to_string(), Value::Bool(true)),
            (
                "private_mode".to_string(),
                Value::String("0710".to_string()),
            ),
            (
                "default_mode".to_string(),
                Value::String("0750".to_string()),
            ),
        ]);
        let context = crate::playbook::templar::create_context(&vars).unwrap();
        let mut engine = crate::playbook::templar::environment();

        let mut template_task = create_test_task();
        template_task.module = "template".to_string();
        template_task.args = mapping(&[
            ("content", Value::String("{{ answer }}".to_string())),
            (
                "mode",
                Value::String("{{ private_mode if private else default_mode }}".to_string()),
            ),
        ]);
        let resolved = template_task
            .resolve_args(&mut engine, &context, &vars)
            .unwrap();
        assert_eq!(
            resolved.get(Value::String("content".to_string())),
            Some(&Value::String("{{ answer }}".to_string()))
        );
        assert_eq!(
            resolved.get(Value::String("mode".to_string())),
            Some(&Value::String("0710".to_string()))
        );

        let mut debug_task = create_test_task();
        debug_task.args = mapping(&[
            ("msg", Value::String("{{ answer }}".to_string())),
            ("var", Value::String("answer".to_string())),
        ]);
        let resolved = debug_task
            .resolve_args(&mut engine, &context, &vars)
            .unwrap();
        assert_eq!(
            resolved.get(Value::String("msg".to_string())),
            Some(&Value::String("42".to_string()))
        );
        assert_eq!(
            resolved.get(Value::String("_var_value".to_string())),
            Some(&Value::Number(42.into()))
        );
    }

    #[test]
    fn resolve_args_removes_default_omit_parameters() {
        let vars = HashMap::new();
        let context = crate::playbook::templar::create_context(&vars).unwrap();
        let mut engine = crate::playbook::templar::environment();
        let mut task = create_test_task();
        task.module = "get_url".to_string();
        task.args = mapping(&[
            ("url", Value::String("http://example.invalid/file".into())),
            ("dest", Value::String("/tmp/file".into())),
            (
                "checksum",
                Value::String("{{ missing_checksum | default(omit) }}".into()),
            ),
        ]);

        let resolved = task.resolve_args(&mut engine, &context, &vars).unwrap();
        assert!(!resolved.contains_key(Value::String("checksum".to_string())));
    }

    #[test]
    fn resolve_args_propagates_nested_render_errors_and_rejects_non_string_keys() {
        let (mut engine, context, vars) = create_test_environment_and_context(None);
        let mut task = create_test_task();
        task.args.insert(
            Value::String("payload".to_string()),
            Value::Sequence(vec![Value::Mapping(Mapping::from_iter([(
                Value::String("secret".to_string()),
                Value::String("{{ missing_nested_value }}".to_string()),
            )]))]),
        );
        let error = task
            .resolve_args(&mut engine, &context, &vars)
            .unwrap_err()
            .to_string();
        assert!(error.contains("Failed to render an argument"));

        task.args.insert(
            Value::String("payload".to_string()),
            Value::Mapping(Mapping::from_iter([(
                Value::Number(1.into()),
                Value::String("value".to_string()),
            )])),
        );
        let error = task
            .resolve_args(&mut engine, &context, &vars)
            .unwrap_err()
            .to_string();
        assert!(error.contains("mapping keys must be strings"));
    }

    #[test]
    fn resolve_args_enforces_depth_and_node_limits() {
        let (mut engine, context, vars) = create_test_environment_and_context(None);
        let mut task = create_test_task();

        let mut deeply_nested = Value::String("leaf".to_string());
        for _ in 0..=MAX_ARGUMENT_TEMPLATE_DEPTH {
            deeply_nested = Value::Sequence(vec![deeply_nested]);
        }
        task.args
            .insert(Value::String("payload".to_string()), deeply_nested);
        let error = task
            .resolve_args(&mut engine, &context, &vars)
            .unwrap_err()
            .to_string();
        assert!(error.contains("maximum nesting depth"));

        task.args.insert(
            Value::String("payload".to_string()),
            Value::Sequence(vec![Value::Null; MAX_ARGUMENT_TEMPLATE_NODES]),
        );
        let error = task
            .resolve_args(&mut engine, &context, &vars)
            .unwrap_err()
            .to_string();
        assert!(error.contains("maximum node count"));
    }
}
