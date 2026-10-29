use anyhow::{bail, Context, Result};
use colored::Colorize;
use log::{debug, error, info};
use serde_yaml::{Mapping, Value};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::inventory::Host;
use crate::playbook::templar::RuntimeVariables;
use crate::playbook::{Handler, Task, TaskResult};

/// Play structure representing a set of tasks to run on hosts
#[derive(Debug, Clone)]
pub struct Play {
    pub strategy: crate::playbook::Strategy,
    pub name: String,
    pub hosts: String,
    pub tasks: Vec<Task>,
    pub pre_tasks: Vec<Task>,
    pub post_tasks: Vec<Task>,
    pub handlers: Vec<Handler>,
    pub vars: Mapping,
    pub role_defaults: Mapping,
    pub role_vars: Mapping,
    pub role_paths: Vec<std::path::PathBuf>,
    pub vars_files: Vec<Value>,
    pub source_file: Option<std::path::PathBuf>,
    pub environment: Mapping,
    /// Distinguishes an explicit play setting from inventory/default values.
    pub become_override: Option<bool>,
    pub is_become: bool, // renamed from 'become' to avoid Rust keyword
    pub become_user: String,
    /// Distinguishes an explicit play user (including `root`) from the default.
    pub become_user_override: Option<String>,
    /// Stop the containing playbook immediately if this play fails.
    pub fail_fast: bool,
    pub serial: Option<Value>,
    pub throttle: Option<usize>,
    pub run_once: Option<bool>,
    pub any_errors_fatal: Option<bool>,
    pub max_fail_percentage: Option<u8>,
    pub force_handlers: bool,
    pub ignore_errors: Option<bool>,
    pub ignore_unreachable: Option<bool>,
    pub no_log: Option<bool>,
    /// Play-level connection override (`local`, `ssh`, or `smart`).
    pub connection: Option<String>,
    /// Ansible gathers facts by default; only an explicit false disables it.
    pub gather_facts: bool,
    /// Static playbook import conditions also gate implicit facts gathering.
    pub import_condition: Option<Value>,
    pub gather_subset: Option<Value>,
    pub gather_timeout: Option<Value>,
    pub tags: Vec<String>,
}

#[derive(Debug, Default, Clone)]
struct HostStats {
    unreachable: usize,
    uncertain: usize,
    ok: usize,
    changed: usize,
    failed: usize,
    skipped: usize,
    ignored: usize,
    rescued: usize,
}

struct PlayStatus<'a> {
    batch_size: usize,
    abort: bool,
    fatal_observed: bool,
    fatal_peers: HashSet<String>,
    hosts: &'a [Host],
    definitions: HashMap<String, HashMap<String, Value>>,
    flushing: usize,
    ran_handlers: HashSet<(String, usize)>,
    active_hosts: HashSet<String>,
    stats: HashMap<String, HostStats>,
    failures: HashMap<String, Vec<TaskFailure>>,
    ended: HashSet<String>,
    notified_handlers: HashSet<(String, String)>,
}
#[derive(Clone)]
struct TaskFailure {
    task: Task,
    result: TaskResult,
    fatal_peer: bool,
    escalated: bool,
}
enum Scheduled {
    Task(Box<Task>, HashSet<String>, bool),
    Rescue(std::rc::Rc<std::cell::RefCell<BlockFrame>>),
    Always(std::rc::Rc<std::cell::RefCell<BlockFrame>>),
    End(std::rc::Rc<std::cell::RefCell<BlockFrame>>),
}
struct BlockFrame {
    members: HashSet<String>,
    rescue: Vec<Task>,
    always: Vec<Task>,
    pending: HashMap<String, Vec<TaskFailure>>,
}

/// Only host runtime data survives plays. Scoped inventory/play/task overlays
/// are rebuilt each time, so persisting a register never leaks play variables.
#[derive(Default)]
pub(crate) struct RunState {
    pub(crate) workers: std::sync::Arc<crate::worker::Runtime>,
    pub(crate) tags: std::sync::Arc<super::TagSelection>,
    pub(crate) extra: std::sync::Arc<HashMap<String, Value>>,
    globals: std::sync::Arc<HashMap<String, Value>>,
    host_state: std::sync::Arc<std::sync::Mutex<HostState>>,
    seen_roles: HashSet<(String, String)>,
    ran_roles: HashSet<(String, String)>,
    stats: HashMap<String, HostStats>,
    stop_play: std::sync::Arc<std::sync::atomic::AtomicBool>,
    throttles: std::sync::Arc<crate::playbook::strategy::Throttles>,
    role_exports: std::sync::Arc<std::sync::Mutex<RoleExports>>,
    pub(crate) abort_book: bool,
    batch_size: usize,
}

#[derive(Default, Clone)]
struct HostState {
    inventory: HashMap<String, HashMap<String, Value>>,
    contexts: HashMap<String, RuntimeVariables>,
    excluded: HashSet<String>,
}
#[derive(Default, Clone)]
struct RoleExports {
    defaults: Mapping,
    variables: Mapping,
}

/// Module arguments belong to the logical host. Connection options are rebuilt
/// for the delegated host from the same variable layers, never copied from an
/// already resolved connection on the original host.
pub(crate) struct Delegation<'a> {
    pub play: &'a Play,
    pub state: &'a RunState,
}
impl Delegation<'_> {
    pub fn connection(
        &self,
        name: &str,
        task: &Task,
        origin: &Host,
    ) -> Result<crate::ssh::config::ConnectionConfig> {
        let (mut scope, variables, known) = {
            let shared = self.state.host_state()?;
            (
                shared
                    .contexts
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| RuntimeVariables {
                        defaults: origin.defaults.clone(),
                        extra: self.state.extra.clone(),
                        ..Default::default()
                    }),
                shared.inventory.get(name).cloned().unwrap_or_default(),
                shared.inventory.contains_key(name),
            )
        };
        let mut host = Host::new(name);
        host.defaults = scope.defaults.clone();
        host.variables = variables;
        if !known && matches!(name, "localhost" | "127.0.0.1" | "::1") {
            host.set_variable("ansible_connection", "local");
        }
        scope.globals = self.state.variable_globals()?;
        let mut effective = self.play.apply_play_settings(task.raw(), &host);
        effective.scope.locals.extend(task.scope.locals.clone());
        let definitions = host.inventory_variables();
        let variables = self.play.variables_for_host(&scope, &definitions, false)?;
        let (definitions, mut data) =
            effective.variable_layers(&host, &scope, &definitions, &variables)?;
        data.insert(
            "inventory_hostname".into(),
            Value::String(origin.name.clone()),
        );
        let variables = crate::playbook::templar::resolve_definitions(&definitions, &data)?;
        let mut connection = effective.connection_config(name, &definitions, &variables)?;
        // A delegated target may run tasks for several logical hosts at once.
        // Their synchronous workers need separate leases; the physical address
        // and identity have already been resolved from the delegated context.
        connection.name = origin.name.clone();
        Ok(connection)
    }
}

impl RunState {
    fn role_exports(&self) -> Result<std::sync::MutexGuard<'_, RoleExports>> {
        self.role_exports
            .lock()
            .map_err(|_| anyhow::anyhow!("Role export state was poisoned"))
    }
    fn fork_for(&self, host: &str) -> Self {
        Self {
            workers: self.workers.clone(),
            tags: self.tags.clone(),
            extra: self.extra.clone(),
            globals: self.globals.clone(),
            host_state: self.host_state.clone(),
            stop_play: self.stop_play.clone(),
            throttles: self.throttles.clone(),
            role_exports: self.role_exports.clone(),
            batch_size: self.batch_size,
            stats: self
                .stats
                .get(host)
                .map(|stats| HashMap::from([(host.to_owned(), stats.clone())]))
                .unwrap_or_default(),
            ..Default::default()
        }
    }
    fn host_state(&self) -> Result<std::sync::MutexGuard<'_, HostState>> {
        self.host_state
            .lock()
            .map_err(|_| anyhow::anyhow!("Host runtime state was poisoned"))
    }
    pub(crate) fn set_execution_modes(&mut self, check: bool, diff: bool) {
        self.globals = std::sync::Arc::new(HashMap::from([
            ("ansible_check_mode".into(), Value::Bool(check)),
            ("ansible_diff_mode".into(), Value::Bool(diff)),
        ]));
    }

    pub(crate) fn prepare(
        &mut self,
        inventory: &crate::inventory::Inventory,
        playbook_dir: &std::path::Path,
        config: Option<&std::path::Path>,
    ) -> Result<()> {
        let groups = inventory.group_members();
        let mut globals = (*self.globals).clone();
        globals.extend(HashMap::from([
            ("groups".into(), serde_yaml::to_value(&groups)?),
            (
                "playbook_dir".into(),
                Value::String(playbook_dir.to_string_lossy().into_owned()),
            ),
        ]));
        if let Some(source) = &inventory.source {
            globals.insert(
                "inventory_file".into(),
                Value::String(source.to_string_lossy().into_owned()),
            );
            globals.insert(
                "inventory_dir".into(),
                Value::String(
                    source
                        .parent()
                        .context("Inventory has no parent")?
                        .to_string_lossy()
                        .into_owned(),
                ),
            );
        }
        if let Some(source) = config {
            globals.insert(
                "ansible_config_file".into(),
                Value::String(source.to_string_lossy().into_owned()),
            );
        }
        self.globals = std::sync::Arc::new(globals);
        let mut shared = self.host_state()?;
        for host in inventory.filter_hosts("all") {
            shared
                .inventory
                .insert(host.name.clone(), host.inventory_variables());
            let scope = shared.contexts.entry(host.name.clone()).or_default();
            scope.extra = self.extra.clone();
            scope.defaults = host.defaults.clone();
            scope.globals = self.globals.clone();
            scope.magic.insert(
                "inventory_hostname".into(),
                Value::String(host.name.clone()),
            );
            scope.magic.insert(
                "group_names".into(),
                serde_yaml::to_value(
                    groups
                        .iter()
                        .filter(|(name, members)| {
                            name.as_str() != "all" && members.contains(&host.name)
                        })
                        .map(|(name, _)| name)
                        .collect::<Vec<_>>(),
                )?,
            );
        }
        Ok(())
    }
    fn hostvars(&self) -> Result<Value> {
        let shared = self.host_state()?.clone();
        let mut snapshot = HashMap::<String, Value>::new();
        for _ in 0..65 {
            let mut next = HashMap::new();
            let hosts = serde_yaml::to_value(&snapshot)?;
            for (name, definitions) in &shared.inventory {
                let scope = shared
                    .contexts
                    .get(name)
                    .context("Missing host variable scope")?;
                let (definitions, mut data) = scope.layers(definitions, &[], None, None)?;
                data.insert("hostvars".into(), hosts.clone());
                let mut values =
                    crate::playbook::templar::resolve_definitions(&definitions, &data)?;
                // Never serialize hostvars into itself. References resolve
                // from the previous snapshot until an acyclic chain settles.
                values.remove("hostvars");
                next.insert(name.clone(), serde_yaml::to_value(values)?);
            }
            if next == snapshot {
                return Ok(serde_yaml::to_value(next)?);
            }
            snapshot = next;
        }
        bail!("Cross-host variable references exceeded the depth limit of 64");
    }
    pub(crate) fn variable_globals(&self) -> Result<std::sync::Arc<HashMap<String, Value>>> {
        // Hostvars is a task-boundary view, as in the pinned executor. A loop
        // updates direct set_fact values locally; qualified hostvars lookups
        // observe those updates on the next task.
        let mut globals = (*self.globals).clone();
        globals.insert("hostvars".into(), self.hostvars()?);
        Ok(std::sync::Arc::new(globals))
    }
    pub(crate) fn scope_for(
        &self,
        host: &str,
        globals: &std::sync::Arc<HashMap<String, Value>>,
    ) -> Result<RuntimeVariables> {
        let mut scope = self
            .host_state()?
            .contexts
            .get(host)
            .context("Missing host scope")?
            .clone();
        scope.globals = globals.clone();
        Ok(scope)
    }
    pub(crate) fn variables_for(
        &self,
        host: &str,
        globals: &std::sync::Arc<HashMap<String, Value>>,
    ) -> Result<HashMap<String, Value>> {
        let shared = self.host_state()?.clone();
        let mut scope = shared
            .contexts
            .get(host)
            .context("Missing host scope")?
            .clone();
        scope.globals = globals.clone();
        let (definitions, data) = scope.layers(
            shared
                .inventory
                .get(host)
                .context("Missing inventory host")?,
            &[],
            None,
            None,
        )?;
        crate::playbook::templar::resolve_definitions(&definitions, &data)
    }
    pub(crate) fn print_recap(&self) {
        println!("\n{}", "PLAY RECAP".bold());
        let mut names: Vec<_> = self.stats.keys().collect();
        names.sort();
        for name in names {
            let stats = &self.stats[name];
            println!(
                "{:<30} : ok={} changed={} unreachable={} failed={} skipped={} ignored={} uncertain={} rescued={}",
                name,
                stats.ok,
                stats.changed,
                stats.unreachable,
                stats.failed,
                stats.skipped,
                stats.ignored,
                stats.uncertain,
                stats.rescued,
            );
        }
    }

    fn merge_result(&mut self, host: &str, task: &Task, result: &TaskResult) -> Result<()> {
        let mut shared = self.host_state()?;
        let HostState {
            contexts,
            inventory,
            ..
        } = &mut *shared;
        let mut updates = Vec::new();
        if let Some(Value::Sequence(items)) = result.values.get("results") {
            for item in items {
                if item.get("skipped").and_then(Value::as_bool) != Some(true) {
                    if let Some(facts) = item.get("ansible_facts") {
                        updates.push((facts, item.get("delegated_to").and_then(Value::as_str)));
                    }
                }
            }
        } else if !result.skipped {
            if let Some(facts) = result.values.get("ansible_facts") {
                updates.push((
                    facts,
                    result.values.get("delegated_to").and_then(Value::as_str),
                ));
            }
        }
        for (facts, delegate) in updates {
            let target = if task.delegate_facts.unwrap_or(false) {
                delegate.unwrap_or(host)
            } else {
                host
            };
            let defaults = contexts
                .get(host)
                .map(|scope| scope.defaults.clone())
                .unwrap_or_default();
            let persistent =
                contexts
                    .entry(target.to_string())
                    .or_insert_with(|| RuntimeVariables {
                        defaults,
                        extra: self.extra.clone(),
                        globals: self.globals.clone(),
                        ..Default::default()
                    });
            persistent.magic.insert(
                "inventory_hostname".into(),
                Value::String(target.to_string()),
            );
            inventory.entry(target.to_string()).or_insert_with(|| {
                if matches!(target, "localhost" | "127.0.0.1" | "::1") {
                    HashMap::from([("ansible_connection".into(), Value::String("local".into()))])
                } else {
                    HashMap::new()
                }
            });
            let facts = facts
                .as_mapping()
                .context("Module ansible_facts must be a mapping")?;
            let variables = facts
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
            match crate::modules::registry::Builtin::lookup(&task.module)? {
                crate::modules::registry::Builtin::SetFact => {
                    crate::inventory::host::merge_variable_layer(
                        &mut persistent.registered,
                        &variables,
                    )
                }
                crate::modules::registry::Builtin::IncludeVars => {
                    crate::inventory::host::merge_variable_layer(
                        &mut persistent.included,
                        &variables,
                    )
                }
                _ => Play::merge_facts(&mut persistent.facts, facts)?,
            }
        }
        if let Some(register) = &task.register {
            let value = Play::registered_value(result);
            contexts
                .entry(host.to_string())
                .or_default()
                .registered
                .insert(register.clone(), value);
        }
        Ok(())
    }
}

impl Play {
    pub(crate) fn validate_strategy(&self) -> Result<()> {
        if self.strategy.engine() != crate::playbook::Strategy::Free {
            return Ok(());
        }
        if self.run_once.unwrap_or(false)
            || self.any_errors_fatal.unwrap_or(false)
            || self.max_fail_percentage.is_some()
        {
            bail!("The free strategy cannot combine run_once, any_errors_fatal, or max_fail_percentage; use linear");
        }
        fn validate(tasks: &[Task]) -> Result<()> {
            for task in tasks {
                let task = task.effective();
                if task.run_once.unwrap_or(false) || task.any_errors_fatal.unwrap_or(false) {
                    bail!(
                        "The free strategy cannot combine run_once or any_errors_fatal; use linear"
                    );
                }
                if let Some(group) = task.group {
                    validate(&group.tasks)?;
                    validate(&group.rescue)?;
                    validate(&group.always)?;
                }
            }
            Ok(())
        }
        validate(&self.pre_tasks)?;
        validate(&self.tasks)?;
        validate(&self.post_tasks)?;
        for handler in &self.handlers {
            validate(std::slice::from_ref(&handler.task))?;
        }
        Ok(())
    }
    pub fn execute(&self, hosts: &[Host]) -> Result<()> {
        self.execute_with_options(hosts, false, 5)
    }

    pub fn execute_with_options(
        &self,
        hosts: &[Host],
        check_mode: bool,
        forks: usize,
    ) -> Result<()> {
        let mut state = RunState::default();
        let result = self
            .clone()
            .execute_in_run(hosts, check_mode, forks, &mut state);
        state.print_recap();
        result
    }

    pub(crate) fn execute_in_run(
        &mut self,
        hosts: &[Host],
        check_mode: bool,
        forks: usize,
        state: &mut RunState,
    ) -> Result<()> {
        self.validate_strategy()?;
        let ordered_hosts = self.strategy.order(hosts)?;
        let hosts = ordered_hosts.as_slice();
        if forks == 0 {
            bail!("Play requires forks to be greater than zero");
        }
        let all_host_names = hosts
            .iter()
            .map(|host| host.name.clone())
            .collect::<Vec<_>>();
        let mut unique = HashSet::new();
        for name in &all_host_names {
            if !unique.insert(name) {
                bail!(
                    "Play '{}' received duplicate inventory host '{}'",
                    self.name,
                    name
                );
            }
        }
        if let Some(directory) = self.source_file.as_ref().and_then(|source| source.parent()) {
            let mut globals = (*state.globals).clone();
            globals.insert(
                "playbook_dir".into(),
                Value::String(directory.to_string_lossy().into_owned()),
            );
            state.globals = std::sync::Arc::new(globals);
            for scope in state.host_state()?.contexts.values_mut() {
                scope.globals = state.globals.clone();
            }
        }
        // Keep the original partition and percentage denominator. A batch
        // containing only previously failed hosts must be skipped, rather than
        // treating those old failures as a newly failed deployment batch.
        let serial = self
            .serial
            .as_ref()
            .map(|serial| -> Result<Value> {
                fn needs_render(value: &Value) -> bool {
                    match value {
                        Value::String(source) => source.contains("{{") || source.contains("{%"),
                        Value::Sequence(items) => items.iter().any(needs_render),
                        _ => false,
                    }
                }
                if !needs_render(serial) {
                    return Ok(serial.clone());
                }
                let runtime = RuntimeVariables {
                    extra: state.extra.clone(),
                    globals: state.variable_globals()?,
                    ..Default::default()
                };
                let overlays = self.variables_for_host(&runtime, &HashMap::new(), false)?;
                let defaults = self
                    .role_defaults
                    .iter()
                    .map(|(name, value)| {
                        (
                            name.as_str().expect("Validated role default").to_owned(),
                            value.clone(),
                        )
                    })
                    .collect();
                let (definitions, data) = runtime.layers(&defaults, &[&overlays], None, None)?;
                let variables = crate::playbook::templar::resolve_definitions(&definitions, &data)?;
                let context = crate::playbook::templar::create_context(&variables)?;
                let args = Mapping::from_iter([(Value::String("serial".into()), serial.clone())]);
                let mut result = crate::playbook::task::render_module_arguments(
                    "scheduling",
                    &args,
                    &mut crate::playbook::templar::environment(),
                    &context,
                    &variables,
                )?;
                result
                    .remove(Value::String("serial".into()))
                    .context("Serial cannot be omitted")
            })
            .transpose()
            .map_err(|error| {
                if self.no_log.unwrap_or(false) {
                    anyhow::anyhow!(
                        "Cannot plan serial batches: output censored because no_log is enabled"
                    )
                } else {
                    error
                }
            })?;
        let batches = crate::playbook::strategy::batches(serial.as_ref(), hosts.len())?;
        state
            .stop_play
            .store(false, std::sync::atomic::Ordering::Release);
        *state.role_exports()? = RoleExports {
            defaults: self.role_defaults.clone(),
            variables: self.role_vars.clone(),
        };
        let mut failures = Vec::new();
        for batch in batches {
            state.batch_size = batch.len();
            let batch = &hosts[batch];
            let excluded = state.host_state()?.excluded.clone();
            let eligible = batch
                .iter()
                .filter(|host| !excluded.contains(&host.name))
                .cloned()
                .collect::<Vec<_>>();
            let batch = eligible.as_slice();
            if batch.is_empty() {
                continue;
            }
            let mut globals = (*state.globals).clone();
            globals.insert(
                "ansible_play_hosts_all".into(),
                serde_yaml::to_value(&all_host_names)?,
            );
            globals.insert(
                "ansible_play_batch".into(),
                serde_yaml::to_value(
                    batch
                        .iter()
                        .filter(|host| !excluded.contains(&host.name))
                        .map(|host| &host.name)
                        .collect::<Vec<_>>(),
                )?,
            );
            state.globals = std::sync::Arc::new(globals);
            for scope in state.host_state()?.contexts.values_mut() {
                scope.globals = state.globals.clone();
            }
            let executed = match self.strategy.engine() {
                crate::playbook::Strategy::Linear => {
                    self.execute_batch(batch, check_mode, forks, state)
                }
                crate::playbook::Strategy::Free => {
                    self.execute_free_batch(batch, check_mode, forks, state)
                }
                crate::playbook::Strategy::Compiled(_) => {
                    bail!("Compiled strategy is not registered")
                }
            };
            if let Err(error) = executed {
                failures.push(format!("{error:#}"));
            }
            let excluded = state.host_state()?.excluded.clone();
            if state.stop_play.load(std::sync::atomic::Ordering::Acquire)
                || (batch.len() == state.batch_size
                    && batch.iter().all(|host| excluded.contains(&host.name)))
            {
                break;
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            bail!(failures.join("; "))
        }
    }

    fn execute_free_batch(
        &mut self,
        hosts: &[Host],
        check_mode: bool,
        forks: usize,
        state: &mut RunState,
    ) -> Result<()> {
        // Host controllers advance independently within the bounded pool.
        // Runtime values, connections and permits are shared; each logical
        // host retains its own task stack, handlers and failure recovery.
        let completed = crate::scheduler::map(hosts, forks, |host| {
            let _vault = state.workers.vault.enter();
            let _extensions = state.workers.extensions.enter();
            let mut play = self.clone();
            let mut run = state.fork_for(&host.name);
            let result = play.execute_batch(std::slice::from_ref(host), check_mode, 1, &mut run);
            (result, run.stats)
        });
        let mut failures = Vec::new();
        for (result, stats) in completed {
            state.stats.extend(stats);
            if let Err(error) = result {
                failures.push(format!("{error:#}"));
            }
        }
        let exports = state.role_exports()?.clone();
        self.role_defaults = exports.defaults;
        self.role_vars = exports.variables;
        if failures.is_empty() {
            Ok(())
        } else {
            bail!(failures.join("; "))
        }
    }

    fn execute_batch(
        &mut self,
        hosts: &[Host],
        check_mode: bool,
        forks: usize,
        state: &mut RunState,
    ) -> Result<()> {
        let start_time = Instant::now();
        state.seen_roles.clear();
        state.ran_roles.clear();
        info!("PLAY [{}] on {} hosts", self.name, hosts.len());
        println!("\n{}", format!("PLAY [{}]", self.name).bold());

        let definitions: HashMap<String, HashMap<String, Value>> = hosts
            .iter()
            .map(|host| (host.name.clone(), host.inventory_variables()))
            .collect();
        {
            let mut shared = state.host_state()?;
            for host in hosts {
                let scope = shared.contexts.entry(host.name.clone()).or_default();
                scope.extra = state.extra.clone();
                scope.defaults = host.defaults.clone();
                scope.globals = state.globals.clone();
                scope
                    .magic
                    .entry("inventory_hostname".into())
                    .or_insert_with(|| Value::String(host.name.clone()));
                shared
                    .inventory
                    .entry(host.name.clone())
                    .or_insert_with(|| host.inventory_variables());
            }
        }
        let excluded = state.host_state()?.excluded.clone();
        let active_hosts: HashSet<String> = hosts
            .iter()
            .filter(|host| !excluded.contains(&host.name))
            .map(|host| host.name.clone())
            .collect();
        let stats: HashMap<String, HostStats> = hosts
            .iter()
            .map(|host| {
                (
                    host.name.clone(),
                    state.stats.remove(&host.name).unwrap_or_default(),
                )
            })
            .collect();
        let failures = HashMap::new();
        let notified_handlers: HashSet<(String, String)> = HashSet::new();

        let mut status = PlayStatus {
            batch_size: state.batch_size,
            abort: false,
            fatal_observed: false,
            fatal_peers: HashSet::new(),
            hosts,
            definitions,
            flushing: 0,
            ran_handlers: HashSet::new(),
            active_hosts,
            stats,
            failures,
            ended: HashSet::new(),
            notified_handlers,
        };
        let limits = crate::playbook::ExecutionOptions {
            check_mode,
            forks,
            ..Default::default()
        };

        let mut sequence = Vec::new();
        if self.gather_facts {
            let gather_task = Task {
                group: None,
                parent: None,
                original: None,
                scope: crate::playbook::TaskScope::default(),
                name: "Gathering Facts".to_string(),
                module: "setup".to_string(),
                args: [
                    ("gather_subset", &self.gather_subset),
                    ("gather_timeout", &self.gather_timeout),
                ]
                .into_iter()
                .filter_map(|(name, value)| {
                    value
                        .as_ref()
                        .map(|value| (Value::String(name.into()), value.clone()))
                })
                .collect(),
                source_file: None,
                become_override: None,
                is_become: false,
                become_user: "root".to_string(),
                become_user_override: None,
                connection: None,
                check_mode: None,
                register: None,
                when: self.import_condition.clone(),
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
                tags: if self.tags.is_empty() {
                    vec!["always".into()]
                } else {
                    self.tags.clone()
                },
                loop_items: None,
                loop_var_name: None,
                index_var_name: None,
                delegate_to: None,
                delegate_facts: None,
                throttle: None,
                run_once: Some(false),
                any_errors_fatal: None,
            };
            sequence.push(gather_task);
        }

        let all_hosts = hosts
            .iter()
            .map(|host| host.name.clone())
            .collect::<HashSet<_>>();
        sequence.extend(self.pre_tasks.clone());
        sequence.push(crate::playbook::parser::flush_task());
        sequence.extend(self.tasks.clone());
        sequence.push(crate::playbook::parser::flush_task());
        sequence.extend(self.post_tasks.clone());
        sequence.push(crate::playbook::parser::flush_task());
        let executed = self.execute_sequence(sequence, all_hosts, state, &limits, &mut status);
        let elapsed = start_time.elapsed();
        let execution_time = format!("{:.2}s", elapsed.as_secs_f64());

        info!("Play '{}' completed in {}", self.name, execution_time);

        state.stats.extend(status.stats);
        println!("\nPlay execution completed in {}", execution_time);

        executed?;
        if status.failures.is_empty() {
            if status.fatal_observed {
                bail!("A task with any_errors_fatal failed; rescue restored the host but the run retains a failure status, as in Ansible 2.21");
            }
            Ok(())
        } else {
            Err(anyhow::anyhow!(status
                .failures
                .iter()
                .flat_map(
                    |(host, failures)| failures.iter().map(move |failure| format!(
                        "Task '{}' failed on host '{}': {}",
                        failure.task.name,
                        host,
                        if failure.result.no_log {
                            "output censored because no_log is enabled"
                        } else {
                            &failure.result.msg
                        }
                    ))
                )
                .collect::<Vec<_>>()
                .join("; ")))
        }
    }

    fn execute_sequence(
        &mut self,
        tasks: Vec<Task>,
        members: HashSet<String>,
        state: &mut RunState,
        options: &crate::playbook::ExecutionOptions,
        status: &mut PlayStatus<'_>,
    ) -> Result<()> {
        let hosts = status.hosts;
        let mut pending = tasks
            .into_iter()
            .map(|task| Scheduled::Task(Box::new(task), members.clone(), false))
            .collect::<std::collections::VecDeque<_>>();
        let mut task_index = 0;
        while let Some(scheduled) = pending.pop_front() {
            if self.strategy.engine() == crate::playbook::Strategy::Free {
                if state.stop_play.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
                let exports = state.role_exports()?.clone();
                self.role_defaults = exports.defaults;
                self.role_vars = exports.variables;
            }
            let (task, selected_hosts, cleanup) = match scheduled {
                Scheduled::Task(task, members, cleanup) => (*task, members, cleanup),
                Scheduled::Rescue(frame) => {
                    let frame = frame.borrow();
                    if !frame.rescue.is_empty() {
                        let mut rescued = HashSet::new();
                        for host in &frame.members {
                            if status.active_hosts.contains(host) {
                                continue;
                            }
                            let Some(errors) = status.failures.get(host) else {
                                continue;
                            };
                            if errors
                                .iter()
                                .any(|error| error.result.unreachable || error.result.uncertain)
                            {
                                continue;
                            }
                            let last = errors.last().expect("Nonempty failures");
                            if !last.fatal_peer {
                                status.fatal_peers.remove(host);
                                let mut shared = state.host_state()?;
                                let scope = shared.contexts.entry(host.clone()).or_default();
                                scope.registered.insert(
                                    "ansible_failed_result".into(),
                                    last.result.registered_value(),
                                );
                                scope.registered.insert(
                                    "ansible_failed_task".into(),
                                    Value::Mapping(Mapping::from_iter([
                                        (
                                            Value::String("name".into()),
                                            Value::String(last.task.name.clone()),
                                        ),
                                        (
                                            Value::String("action".into()),
                                            Value::String(last.task.module.clone()),
                                        ),
                                        (
                                            Value::String("args".into()),
                                            Value::Mapping(last.task.args.clone()),
                                        ),
                                    ])),
                                );
                            }
                            if status.flushing == 0 {
                                let origin = hosts
                                    .iter()
                                    .find(|candidate| candidate.name == *host)
                                    .context("Missing rescue host")?;
                                let mut selected = false;
                                for task in &frame.rescue {
                                    selected |= self.task_selected(task, origin, state)?;
                                }
                                if !selected {
                                    continue;
                                }
                            }
                            let errors = status.failures.remove(host).expect("Observed failure");
                            let stats = status
                                .stats
                                .get_mut(host)
                                .context("Missing rescue statistics")?;
                            let counted = errors
                                .iter()
                                .filter(|error| !error.result.inherited_result)
                                .count();
                            stats.failed = stats.failed.saturating_sub(counted);
                            stats.rescued += counted;
                            status.active_hosts.insert(host.clone());
                            if !status.fatal_peers.contains(host) {
                                state.host_state()?.excluded.remove(host);
                            }
                            rescued.insert(host.clone());
                        }
                        for task in frame.rescue.iter().rev() {
                            pending.push_front(Scheduled::Task(
                                Box::new(task.clone()),
                                rescued.clone(),
                                true,
                            ));
                        }
                    }
                    continue;
                }
                Scheduled::Always(frame) => {
                    let mut frame = frame.borrow_mut();
                    let mut eligible = HashSet::new();
                    for host in frame.members.clone() {
                        if status.ended.contains(&host) {
                            continue;
                        }
                        if status.failures.get(&host).is_some_and(|errors| {
                            errors
                                .iter()
                                .any(|error| error.result.unreachable || error.result.uncertain)
                        }) {
                            continue;
                        }
                        if let Some(errors) = status.failures.remove(&host) {
                            frame.pending.insert(host.clone(), errors);
                        }
                        status.active_hosts.insert(host.clone());
                        if !status.fatal_peers.contains(&host) {
                            state.host_state()?.excluded.remove(&host);
                        }
                        eligible.insert(host);
                    }
                    for task in frame.always.iter().rev() {
                        pending.push_front(Scheduled::Task(
                            Box::new(task.clone()),
                            eligible.clone(),
                            true,
                        ));
                    }
                    continue;
                }
                Scheduled::End(frame) => {
                    for (host, mut errors) in std::mem::take(&mut frame.borrow_mut().pending) {
                        if let Some(mut cleanup) = status.failures.remove(&host) {
                            errors.append(&mut cleanup);
                        }
                        status.failures.insert(host.clone(), errors);
                        status.active_hosts.remove(&host);
                        state.host_state()?.excluded.insert(host);
                    }
                    self.apply_failure_policy(state, status, &pending)?;
                    continue;
                }
            };
            let task = &task;
            if status.abort && !cleanup && status.flushing == 0 {
                if task.scope.boundary == Some(crate::playbook::task::Boundary::Flush) {
                    self.flush_handlers(state, options, status)?;
                }
                continue;
            }
            if let Some(group) = &task.group {
                let members = selected_hosts
                    .iter()
                    .filter(|host| status.active_hosts.contains(*host))
                    .cloned()
                    .collect::<HashSet<_>>();
                let bind = |tasks: &[Task]| task.bind_children(tasks);
                let frame = std::rc::Rc::new(std::cell::RefCell::new(BlockFrame {
                    members: members.clone(),
                    rescue: bind(&group.rescue),
                    always: bind(&group.always),
                    pending: HashMap::new(),
                }));
                pending.push_front(Scheduled::End(frame.clone()));
                pending.push_front(Scheduled::Always(frame.clone()));
                pending.push_front(Scheduled::Rescue(frame));
                for child in bind(&group.tasks).into_iter().rev() {
                    pending.push_front(Scheduled::Task(Box::new(child), members.clone(), cleanup));
                }
                continue;
            }
            if task.scope.boundary == Some(crate::playbook::task::Boundary::Flush) {
                self.flush_handlers(state, options, status)?;
                continue;
            }
            if let (Some(boundary), Some(role)) = (task.scope.boundary, task.scope.role_id.as_ref())
            {
                let eligible = selected_hosts
                    .iter()
                    .filter(|host| {
                        status.active_hosts.contains(*host)
                            && (task.scope.allow_duplicates
                                || !state.seen_roles.contains(&((*host).clone(), role.clone())))
                    })
                    .cloned()
                    .collect::<HashSet<_>>();
                match boundary {
                    crate::playbook::task::Boundary::Start => {
                        for scheduled in &mut pending {
                            let Scheduled::Task(child, members, _) = scheduled else {
                                continue;
                            };
                            members.retain(|host| eligible.contains(host));
                            if child.scope.boundary == Some(crate::playbook::task::Boundary::End)
                                && child.scope.role_id.as_ref() == Some(role)
                            {
                                break;
                            }
                        }
                    }
                    crate::playbook::task::Boundary::Flush => {
                        unreachable!("Flush handled before role boundaries")
                    }
                    crate::playbook::task::Boundary::End => {
                        for host in selected_hosts
                            .iter()
                            .filter(|host| status.active_hosts.contains(*host))
                        {
                            if state.ran_roles.contains(&(host.clone(), role.clone())) {
                                state.seen_roles.insert((host.clone(), role.clone()));
                            }
                        }
                    }
                }
                continue;
            }
            if status.flushing == 0 {
                let mut tags = task.effective().tags;
                tags.extend(self.tags.clone());
                if state.tags.literal(&tags) == Some(false) {
                    continue;
                }
            }
            let mut expansions = Vec::<(Vec<Task>, Vec<Handler>, HashSet<String>)>::new();
            let mut expansion_keys = HashMap::<String, usize>::new();
            if status.active_hosts.is_empty() {
                debug!(
                    "Stopping play '{}' before task {} because no active hosts remain",
                    self.name,
                    task_index + 1
                );
                continue;
            }
            debug!(
                "Executing task {} of {}: {}",
                task_index + 1,
                self.tasks.len(),
                task.name
            );

            // 更接近ansible风格的任务标题
            let title_width = task.name.chars().count() + 8;
            println!(
                "\nTASK [{}] {}",
                task.name,
                "*".repeat(80usize.saturating_sub(title_width)).dimmed()
            );

            let runnable_hosts: Vec<&Host> = hosts
                .iter()
                .filter(|host| {
                    status.active_hosts.contains(&host.name) && selected_hosts.contains(&host.name)
                })
                .collect();
            let task_results = Self::execute_task_on_hosts(
                self,
                task,
                runnable_hosts,
                state,
                status.flushing > 0,
                &status.definitions,
                options,
            );

            for (host, effective_task, task_result) in task_results {
                if task_result.as_ref().is_ok_and(|result| result.filtered) {
                    continue;
                }
                let mut result = match task_result {
                    Ok(result) => result,
                    Err(error) => {
                        if effective_task.no_log.unwrap_or(false) || state.workers.vault.used() {
                            error!("A no_log task failed on host {}", host.name);
                        } else {
                            error!("Task execution failed on host {}: {}", host.name, error);
                        }
                        let mut result = TaskResult::new(&host.name);
                        result.failed = true;
                        result.no_log =
                            effective_task.no_log.unwrap_or(false) || state.workers.vault.used();
                        result.msg = if effective_task.no_log.unwrap_or(false)
                            || state.workers.vault.used()
                        {
                            "output censored because no_log is enabled".to_string()
                        } else {
                            format!("Task execution error: {}", error)
                        };
                        crate::playbook::task::print_task_result(
                            &host.name,
                            &effective_task.name,
                            &result,
                            "0.00s",
                        );
                        result
                    }
                };

                if !result.failed && !result.skipped {
                    for request in std::mem::take(&mut result.includes) {
                        let compiled = (|| -> Result<()> {
                            let key = serde_json::to_vec(&serde_json::to_value((
                                request.kind as u8,
                                request.iteration,
                                &request.args,
                                &request.parent.vars,
                                &request.parent.scope,
                                &request.parent.source_file,
                            ))?)?;
                            let key = rustsible_protocol::sha256(&mut key.as_slice())?;
                            let index = if let Some(index) = expansion_keys.get(&key) {
                                *index
                            } else {
                                let tasks = crate::playbook::parser::expand_tasks(
                                    &request,
                                    &crate::playbook::parser::ParseOptions {
                                        variables: (*state.extra).clone(),
                                        role_paths: self.role_paths.clone(),
                                        playbook_dir: self
                                            .source_file
                                            .as_ref()
                                            .and_then(|source| source.parent())
                                            .map(std::path::Path::to_path_buf),
                                        ..Default::default()
                                    },
                                )?;
                                if self.strategy.engine() == crate::playbook::Strategy::Free {
                                    let mut selected = self.clone();
                                    selected.pre_tasks.clear();
                                    selected.post_tasks.clear();
                                    selected.tasks = tasks.tasks.clone();
                                    selected.handlers = tasks.handlers.clone();
                                    selected.validate_strategy()?;
                                }
                                let index = expansions.len();
                                expansion_keys.insert(key, index);
                                if self.strategy.engine() == crate::playbook::Strategy::Free {
                                    let mut exports = state.role_exports()?;
                                    crate::vars::merge(&mut exports.defaults, tasks.defaults);
                                    crate::vars::merge(&mut exports.variables, tasks.variables);
                                    self.role_defaults = exports.defaults.clone();
                                    self.role_vars = exports.variables.clone();
                                } else {
                                    crate::vars::merge(&mut self.role_defaults, tasks.defaults);
                                    crate::vars::merge(&mut self.role_vars, tasks.variables);
                                }
                                expansions.push((tasks.tasks, tasks.handlers, HashSet::new()));
                                index
                            };
                            expansions[index].2.insert(host.name.clone());
                            Ok(())
                        })();
                        if let Err(error) = compiled {
                            result.failed = true;
                            result.notifications.clear();
                            result.msg = if result.no_log {
                                "output censored because no_log is enabled".into()
                            } else {
                                format!("Cannot load selected include: {error:#}")
                            };
                            crate::playbook::task::print_task_result(
                                &host.name,
                                &effective_task.name,
                                &result,
                                "0.00s",
                            );
                            break;
                        }
                    }
                }
                if result
                    .notifications
                    .iter()
                    .any(|notice| self.handler_targets(&host.name, notice, state).is_empty())
                {
                    result.failed = true;
                    result.notifications.clear();
                    result.msg =
                        "Handler notification has no matching definition; check task notify names"
                            .into();
                    crate::playbook::task::print_task_result(
                        &host.name,
                        &effective_task.name,
                        &result,
                        "0.00s",
                    );
                }
                state.merge_result(&host.name, &effective_task, &result)?;
                if let Some(action) = &result.meta {
                    match action.as_str() {
                        "reset_connection" => state.workers.reset(&host.name)?,
                        "flush_handlers" => self.flush_handlers(state, options, status)?,
                        "clear_host_errors" => {
                            status.fatal_peers.clear();
                            if let Some(targets) = state
                                .globals
                                .get("ansible_play_hosts_all")
                                .and_then(Value::as_sequence)
                            {
                                for name in targets.iter().filter_map(Value::as_str) {
                                    state.host_state()?.excluded.remove(name);
                                }
                            }
                        }
                        "end_host" => {
                            status.active_hosts.remove(&host.name);
                            status.ended.insert(host.name.clone());
                        }
                        "end_play" => {
                            state
                                .stop_play
                                .store(true, std::sync::atomic::Ordering::Release);
                            status.active_hosts.clear();
                            status
                                .ended
                                .extend(hosts.iter().map(|host| host.name.clone()));
                            pending.clear();
                            status.notified_handlers.clear();
                        }
                        _ => unreachable!("Meta action validated before selection"),
                    }
                    continue;
                }

                if !result.skipped && !result.inherited_result {
                    if let Some(role) = &effective_task.scope.role_id {
                        state.ran_roles.insert((host.name.clone(), role.clone()));
                    }
                }

                if !result.notifications.is_empty() {
                    for handler_name in &result.notifications {
                        debug!(
                            "Handler '{}' notified on host '{}' by task '{}'",
                            handler_name, host.name, effective_task.name
                        );
                        status
                            .notified_handlers
                            .insert((host.name.clone(), handler_name.clone()));
                    }
                }

                let host_stats = status.stats
                    .get_mut(&host.name)
                    .with_context(|| {
                        format!(
                            "Internal play state error: statistics for host '{}' are missing after task '{}'",
                            host.name, effective_task.name
                        )
                    })?;
                if result.failed {
                    if !result.uncertain
                        && ((result.unreachable
                            && effective_task.ignore_unreachable.unwrap_or(false))
                            || (!result.unreachable
                                && effective_task.ignore_errors.unwrap_or(false)))
                    {
                        host_stats.ignored += usize::from(!result.inherited_result);
                    } else {
                        status.fatal_observed |= effective_task.any_errors_fatal.unwrap_or(false);
                        if result.uncertain {
                            host_stats.uncertain += usize::from(!result.inherited_result);
                        } else if result.unreachable {
                            host_stats.unreachable += usize::from(!result.inherited_result);
                        } else {
                            host_stats.failed += usize::from(!result.inherited_result);
                        }
                        status.active_hosts.remove(&host.name);
                        state.host_state()?.excluded.insert(host.name.clone());
                        status
                            .failures
                            .entry(host.name.clone())
                            .or_default()
                            .push(TaskFailure {
                                task: effective_task.clone(),
                                result: result.clone(),
                                fatal_peer: false,
                                escalated: false,
                            });
                    }
                } else if result.skipped {
                    host_stats.skipped += usize::from(!result.inherited_result);
                } else {
                    host_stats.ok += usize::from(!result.inherited_result);
                    if result.changed {
                        host_stats.changed += usize::from(!result.inherited_result);
                    }
                }
            }
            self.apply_failure_policy(state, status, &pending)?;
            for (_, handlers, members) in &mut expansions {
                for mut handler in handlers.drain(..) {
                    handler.hosts = Some(members.clone());
                    self.handlers.push(handler);
                }
            }
            for (tasks, _, members) in expansions.into_iter().rev() {
                for task in tasks.into_iter().rev() {
                    pending.push_front(Scheduled::Task(Box::new(task), members.clone(), cleanup));
                }
            }
            task_index += 1;
        }

        Ok(())
    }

    /// Rescue gets the opportunity to handle module failures before a failure
    /// limit aborts the deployment. Cleanup remains scheduled after an abort.
    fn apply_failure_policy(
        &self,
        state: &mut RunState,
        status: &mut PlayStatus<'_>,
        pending: &std::collections::VecDeque<Scheduled>,
    ) -> Result<()> {
        let mut triggers = Vec::new();
        for (host, errors) in &mut status.failures {
            for error in errors {
                if !error.escalated
                    && error
                        .task
                        .any_errors_fatal
                        .unwrap_or(self.any_errors_fatal.unwrap_or(false))
                {
                    error.escalated = true;
                    triggers.push((host.clone(), error.task.clone()));
                }
            }
        }
        for (origin, task) in triggers {
            for host in std::mem::take(&mut status.active_hosts) {
                let mut result = TaskResult::new(&host);
                result.failed = true;
                result.inherited_result = true;
                result.msg = format!("Execution stopped because a task with any_errors_fatal failed on host '{origin}'");
                status.fatal_peers.insert(host.clone());
                state.host_state()?.excluded.insert(host.clone());
                status.failures.entry(host).or_default().push(TaskFailure {
                    task: task.clone(),
                    result,
                    fatal_peer: true,
                    escalated: true,
                });
            }
        }
        let unhandled = status.failures.iter().filter(|(host, errors)| {
            let terminal = errors.iter().any(|error| error.result.unreachable || error.result.uncertain);
            terminal || !pending.iter().any(|event| matches!(event,
                Scheduled::Rescue(frame) if !frame.borrow().rescue.is_empty() && frame.borrow().members.contains(*host)
            ))
        }).collect::<Vec<_>>();
        let fatal = unhandled.iter().any(|(_, errors)| {
            errors.iter().any(|error| {
                error
                    .task
                    .any_errors_fatal
                    .unwrap_or(self.any_errors_fatal.unwrap_or(false))
            })
        });
        let exceeded = self.max_fail_percentage.is_some_and(|percentage| {
            unhandled.len() as u128 * 100 > status.batch_size as u128 * percentage as u128
        });
        if fatal || exceeded {
            status.abort = true;
            state
                .stop_play
                .store(true, std::sync::atomic::Ordering::Release);
            state.abort_book = true;
        }
        Ok(())
    }

    fn handler_name(&self, handler: &Handler, state: &RunState) -> Result<String> {
        if !handler.task.name.contains("{{") && !handler.task.name.contains("{%") {
            return Ok(handler.task.name.clone());
        }
        let task = handler.task.effective();
        // Handler names are evaluated without a host, as in the reference.
        // Role/include parameters and loop bindings remain lexical context.
        let runtime = RuntimeVariables {
            extra: state.extra.clone(),
            globals: state.variable_globals()?,
            ..Default::default()
        };
        let mut defaults = self.role_defaults.clone();
        crate::vars::merge(&mut defaults, task.scope.defaults.clone());
        let definitions = defaults
            .into_iter()
            .map(|(name, value)| (name.as_str().expect("Validated default").to_owned(), value))
            .collect();
        let mut params = task.scope.role_params.clone();
        crate::vars::merge(&mut params, task.scope.params.clone());
        let (definitions, mut data) = runtime.layers(
            &definitions,
            &[
                &self.vars,
                &self.role_vars,
                &task.scope.role_vars,
                &task.vars,
            ],
            None,
            Some(&params),
        )?;
        data.extend(task.scope.locals.clone());
        let variables = crate::playbook::templar::resolve_definitions(&definitions, &data)?;
        let context = crate::playbook::templar::create_context(&variables)?;
        crate::playbook::templar::render_value(
            &task.name,
            &mut crate::playbook::templar::environment(),
            &context,
            true,
        )?
        .as_str()
        .map(str::to_owned)
        .context("Handler name must be a string")
    }
    fn handler_targets(&mut self, host: &str, notice: &str, state: &RunState) -> Vec<usize> {
        for index in 0..self.handlers.len() {
            if self.handlers[index].cached_name.is_none() {
                if let Ok(name) = self.handler_name(&self.handlers[index], state) {
                    self.handlers[index].cached_name = Some(name);
                }
            }
        }
        let eligible = |handler: &Handler| {
            handler
                .hosts
                .as_ref()
                .is_none_or(|hosts| hosts.contains(host))
        };
        let qualified = |handler: &Handler, name: &str| {
            handler
                .task
                .scope
                .role_name
                .as_ref()
                .map(|role| format!("{role} : {name}"))
                .is_some_and(|name| name == notice)
        };
        let mut selected = Vec::new();
        for (index, handler) in self
            .handlers
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, handler)| eligible(handler))
        {
            if let Some(name) = &handler.cached_name {
                if name == notice || qualified(handler, name) {
                    selected.push(index);
                    break;
                }
            }
        }
        let mut seen = HashSet::new();
        for (index, handler) in self
            .handlers
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, handler)| eligible(handler))
        {
            let name = handler.cached_name.as_ref().unwrap_or(&handler.task.name);
            if handler
                .listen
                .iter()
                .any(|topic| topic == notice || qualified(handler, topic))
                && seen.insert(name.clone())
            {
                selected.push(index);
            }
        }
        selected.sort_unstable();
        selected.dedup();
        selected
    }

    fn flush_handlers(
        &mut self,
        state: &mut RunState,
        options: &crate::playbook::ExecutionOptions,
        status: &mut PlayStatus<'_>,
    ) -> Result<()> {
        status.flushing += 1;
        loop {
            let mut selected = HashMap::<usize, HashSet<String>>::new();
            for (host, notice) in status.notified_handlers.clone() {
                if status.ended.contains(&host) {
                    continue;
                }
                let terminal = status.failures.get(&host).is_some_and(|errors| {
                    errors
                        .iter()
                        .any(|error| error.result.unreachable || error.result.uncertain)
                });
                if !status.active_hosts.contains(&host) && (!self.force_handlers || terminal) {
                    continue;
                }
                for index in self.handler_targets(&host, &notice, state) {
                    if !status.ran_handlers.contains(&(host.clone(), index)) {
                        selected.entry(index).or_default().insert(host.clone());
                    }
                }
            }
            let Some(index) = selected.keys().min().copied() else {
                break;
            };
            let members = selected.remove(&index).expect("Selected handler");
            for host in &members {
                status.ran_handlers.insert((host.clone(), index));
            }
            let mut task = self.handlers[index].task.clone();
            if let Some(name) = &self.handlers[index].cached_name {
                task.name = name.clone();
            }
            println!("\n{}", format!("RUNNING HANDLER [{}]", task.name).bold());
            let mut restore = HashMap::new();
            for host in &members {
                if let Some(errors) = status.failures.remove(host) {
                    restore.insert(host.clone(), errors);
                }
                status.active_hosts.insert(host.clone());
                state.host_state()?.excluded.remove(host);
            }
            self.execute_sequence(vec![task], members, state, options, status)?;
            state
                .host_state()?
                .excluded
                .extend(status.fatal_peers.iter().cloned());
            for (host, mut errors) in restore {
                if let Some(mut extra) = status.failures.remove(&host) {
                    errors.append(&mut extra);
                }
                status.failures.insert(host.clone(), errors);
                status.active_hosts.remove(&host);
                state.host_state()?.excluded.insert(host);
            }
        }
        status.flushing -= 1;
        if status.flushing == 0 {
            status.notified_handlers.clear();
            status.ran_handlers.clear();
        }
        Ok(())
    }

    /// Execute one task concurrently across hosts, while keeping the task
    /// sequence itself ordered. Each bounded worker takes the next available
    /// host immediately, so a slow host cannot stall an otherwise free slot.
    fn execute_task_on_hosts<'a>(
        play: &Play,
        task: &Task,
        hosts: Vec<&'a Host>,
        state: &RunState,
        handler: bool,
        definitions: &HashMap<String, HashMap<String, Value>>,
        options: &crate::playbook::ExecutionOptions,
    ) -> Vec<(&'a Host, Task, Result<TaskResult>)> {
        let check_mode = options.check_mode;
        let forks = options.forks;
        let runtime = &state.workers;
        let globals = state.variable_globals();
        let Some(first) = hosts.first() else {
            return Vec::new();
        };
        let controls = play.apply_play_settings(task, first);
        let run_once = controls.run_once.unwrap_or(false);
        let concurrency = controls
            .throttle
            .filter(|limit| *limit > 0)
            .unwrap_or(forks)
            .min(forks);
        let selected = if run_once {
            &hosts[..1]
        } else {
            hosts.as_slice()
        };
        let mut results = crate::scheduler::map(selected, concurrency, |&host| {
            let _vault = state.workers.vault.enter();
            let _extensions = state.workers.extensions.enter();
            let effective_task = play.apply_play_settings(task, host);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _permit = if play.strategy.engine() == crate::playbook::Strategy::Free {
                    if let Some(limit) = effective_task.throttle.filter(|limit| *limit > 0) {
                        let identity = serde_json::to_vec(&serde_json::to_value((
                            &effective_task.source_file,
                            &effective_task.module,
                            &effective_task.name,
                            &effective_task.args,
                            &effective_task.scope,
                            &effective_task.throttle,
                        ))?)?;
                        let key = rustsible_protocol::sha256(&mut identity.as_slice())?;
                        Some(state.throttles.acquire(key, limit)?)
                    } else {
                        None
                    }
                } else {
                    None
                };
                let scope = state.host_state()?.contexts.get(&host.name).cloned();
                match (scope, definitions.get(&host.name)) {
                    (Some(mut vars), Some(definitions)) => {
                        vars.globals = globals
                            .as_ref()
                            .map_err(|error| {
                                anyhow::anyhow!("Cannot construct hostvars: {error:#}")
                            })?
                            .clone();
                        let implicit_gather = task.module == "setup" && task.source_file.is_none();
                        let variables =
                            play.variables_for_host(&vars, definitions, implicit_gather)?;
                        effective_task.execute_with_definitions(
                            host,
                            &vars,
                            definitions,
                            &variables,
                            check_mode,
                            &crate::playbook::task::TaskExecution {
                                workers: runtime,
                                tags: (!handler).then_some(state.tags.as_ref()),
                                delegation: Some(Delegation { play, state }),
                            },
                        )
                    }
                    _ => Err(anyhow::anyhow!(
                        "Internal error: no variable context for host '{}'",
                        host.name
                    )),
                }
            }))
            .unwrap_or_else(|_| {
                Err(anyhow::anyhow!(
                    "Task worker panicked while processing host '{}'",
                    host.name
                ))
            });
            let result = result.map_err(|error| {
                if let Some(source) = &effective_task.source_file {
                    anyhow::anyhow!(
                        "{}: task '{}': {error:#}",
                        source.display(),
                        effective_task.name
                    )
                } else {
                    error
                }
            });
            (host, effective_task, result)
        });
        if run_once {
            let (_, task, result) = &results[0];
            let mut copies = Vec::new();
            for host in hosts.iter().skip(1) {
                let result = match result {
                    Ok(result) => {
                        let mut result = result.clone();
                        result.host = host.name.clone();
                        result.inherited_result = true;
                        result.notifications.clear();
                        result.includes.clear();
                        result.meta = None;
                        Ok(result)
                    }
                    Err(error) => {
                        let mut result = TaskResult::new(&host.name);
                        result.inherited_result = true;
                        result.failed = true;
                        result.no_log = task.no_log.unwrap_or(false);
                        result.msg = if result.no_log {
                            "output censored because no_log is enabled".into()
                        } else {
                            format!("Task execution error: {error:#}")
                        };
                        Ok(result)
                    }
                };
                copies.push((*host, task.clone(), result));
            }
            results.extend(copies);
        }
        results
    }

    pub(crate) fn task_selected(&self, task: &Task, host: &Host, state: &RunState) -> Result<bool> {
        if let Some(group) = &task.group {
            for tasks in [&group.tasks, &group.rescue, &group.always] {
                for child in task.bind_children(tasks) {
                    if self.task_selected(&child, host, state)? {
                        return Ok(true);
                    }
                }
            }
            return Ok(false);
        }
        let task = self.apply_play_settings(task, host);
        if let Some(value) = state.tags.literal(&task.tags) {
            return Ok(value);
        }
        let globals = state.variable_globals()?;
        let runtime = state.scope_for(&host.name, &globals)?;
        let definitions = host.inventory_variables();
        let variables = self.variables_for_host(&runtime, &definitions, true)?;
        let (definitions, data) = task.variable_layers(host, &runtime, &definitions, &variables)?;
        state.tags.selected(
            &task.tags,
            &crate::playbook::templar::resolve_definitions(&definitions, &data)?,
        )
    }
    pub(crate) fn apply_play_settings(&self, task: &Task, host: &Host) -> Task {
        let mut effective_task = task.effective();
        effective_task.tags.extend(self.tags.clone());
        effective_task.ignore_errors = effective_task.ignore_errors.or(self.ignore_errors);
        effective_task.ignore_unreachable = effective_task
            .ignore_unreachable
            .or(self.ignore_unreachable);
        effective_task.no_log = effective_task.no_log.or(self.no_log);
        effective_task.throttle = effective_task.throttle.or(self.throttle);
        effective_task.run_once = effective_task.run_once.or(self.run_once);
        effective_task.any_errors_fatal = effective_task.any_errors_fatal.or(self.any_errors_fatal);
        let mut defaults = self.role_defaults.clone();
        crate::vars::merge(&mut defaults, effective_task.scope.defaults.clone());
        effective_task.scope.defaults = defaults;
        effective_task.become_override = effective_task.become_override.or(self.become_override);
        effective_task.become_user_override = effective_task
            .become_user_override
            .clone()
            .or_else(|| self.become_user_override.clone());
        let mut environment = self.environment.clone();
        environment.extend(effective_task.environment.clone());
        effective_task.environment = environment;
        effective_task.is_become = effective_task
            .become_override
            .or(self.become_override)
            .or_else(|| host.get_become())
            .unwrap_or(false);
        effective_task.become_user = effective_task
            .become_user_override
            .clone()
            .or_else(|| self.become_user_override.clone())
            .or_else(|| host.get_become_user().map(str::to_owned))
            .unwrap_or_else(|| "root".to_string());
        if effective_task.connection.is_none() {
            effective_task.connection = self.connection.clone();
        }
        effective_task
    }

    pub(crate) fn variables_for_host(
        &self,
        runtime: &RuntimeVariables,
        definitions: &HashMap<String, Value>,
        allow_unresolved: bool,
    ) -> Result<Mapping> {
        let mut variables = self.vars.clone();
        let base = self
            .source_file
            .as_ref()
            .and_then(|path| path.parent())
            .map(std::path::Path::to_path_buf)
            .unwrap_or(std::env::current_dir()?);
        let mut engine = crate::playbook::templar::environment();
        for entry in &self.vars_files {
            let mut keywords = HashMap::new();
            if let Some(value) = self.become_override {
                keywords.insert("ansible_become".into(), Value::Bool(value));
            }
            if let Some(value) = &self.become_user_override {
                keywords.insert("ansible_become_user".into(), Value::String(value.clone()));
            }
            let (definitions, data) =
                runtime.layers(definitions, &[&variables], Some(&keywords), None)?;
            let resolved = crate::playbook::templar::resolve_definitions(&definitions, &data)?;
            let context = crate::playbook::templar::create_context(&resolved)?;
            let candidates = entry
                .as_sequence()
                .map(Vec::as_slice)
                .unwrap_or(std::slice::from_ref(entry));
            let mut selected = None;
            let mut unresolved = false;
            for candidate in candidates {
                let value =
                    match crate::playbook::templar::render_value(
                        candidate
                            .as_str()
                            .context("vars_files paths must be strings")?,
                        &mut engine,
                        &context,
                        true,
                    ) {
                        Ok(value) => value,
                        Err(error)
                            if allow_unresolved
                                && error.downcast_ref::<minijinja::Error>().is_some_and(
                                    |error| error.kind() == minijinja::ErrorKind::UndefinedError,
                                ) =>
                        {
                            unresolved = true;
                            continue;
                        }
                        Err(error) => return Err(error),
                    };
                let path = value
                    .as_str()
                    .context("vars_files path must resolve to a string")?;
                let path = base.join(path);
                if path.exists() {
                    selected = Some(path);
                    break;
                }
            }
            // Static files can set connection variables before gathering;
            // fact-dependent paths wait until their facts actually exist.
            if selected.is_none() && unresolved {
                continue;
            }
            let selected = selected.context("No vars_files candidate exists")?;
            crate::vars::merge(&mut variables, crate::vars::load(&selected)?);
        }
        crate::vars::merge(&mut variables, self.role_vars.clone());
        Ok(variables)
    }

    pub(crate) fn merge_facts(context: &mut HashMap<String, Value>, facts: &Mapping) -> Result<()> {
        let mut merged = context
            .get("ansible_facts")
            .and_then(Value::as_mapping)
            .cloned()
            .unwrap_or_default();
        for (key, value) in facts {
            let Value::String(key) = key else {
                bail!("Fact names must be strings");
            };
            merged.insert(
                Value::String(key.strip_prefix("ansible_").unwrap_or(key).to_owned()),
                value.clone(),
            );
            context.insert(key.clone(), value.clone());
        }
        context.insert("ansible_facts".to_string(), Value::Mapping(merged));
        Ok(())
    }

    fn registered_value(result: &TaskResult) -> Value {
        result.registered_value()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::create_test_host;
    use serde_yaml::Mapping;
    use std::fs;
    use tempfile::tempdir;

    fn create_local_host() -> Host {
        create_test_host("localhost", "localhost", 22, None, None)
    }

    fn create_test_play() -> Play {
        Play {
            strategy: crate::playbook::Strategy::Linear,
            name: "Test Play".to_string(),
            hosts: "localhost".to_string(),
            tasks: Vec::new(),
            pre_tasks: Vec::new(),
            post_tasks: Vec::new(),
            handlers: Vec::new(),
            vars: Mapping::new(),
            role_defaults: Mapping::new(),
            role_vars: Mapping::new(),
            role_paths: Vec::new(),
            environment: Mapping::new(),
            vars_files: Vec::new(),
            source_file: None,
            become_override: None,
            is_become: false,
            become_user: "root".to_string(),
            become_user_override: None,
            fail_fast: false,
            serial: None,
            throttle: None,
            run_once: None,
            any_errors_fatal: None,
            max_fail_percentage: None,
            force_handlers: false,
            ignore_errors: None,
            ignore_unreachable: None,
            no_log: None,
            connection: None,
            gather_facts: false,
            import_condition: None,
            gather_subset: None,
            gather_timeout: None,
            tags: Vec::new(),
        }
    }

    fn create_command_task(name: &str, command: &str) -> Task {
        let mut args = Mapping::new();
        args.insert(
            Value::String("_raw_params".to_string()),
            Value::String(command.to_string()),
        );

        Task {
            group: None,
            parent: None,
            original: None,
            scope: crate::playbook::TaskScope::default(),
            name: name.to_string(),
            module: "command".to_string(),
            args,
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

    #[test]
    fn play_vars_override_inventory_and_preserve_typed_values() {
        let mut host = create_local_host();
        host.set_variable("priority", "inventory");
        host.set_typed_variable(
            "items",
            Value::Sequence(vec![Value::String("one".to_string())]),
        );
        let mut play = create_test_play();
        play.vars.insert(
            Value::String("priority".to_string()),
            Value::String("play".to_string()),
        );

        let vars = host.all_variables();
        let mut task = create_command_task("typed variables", "true");
        task.module = "debug".into();
        task.args = Mapping::from_iter([(
            Value::String("msg".into()),
            Value::String("{{ priority }}:{{ items | join(',') }}".into()),
        )]);
        let result = task
            .execute_with_definitions(
                &host,
                &RuntimeVariables::default(),
                &vars,
                &play.vars,
                false,
                &crate::playbook::task::TaskExecution::new(&crate::worker::Runtime::default()),
            )
            .unwrap();
        assert_eq!(result.msg, "play:one");
        assert!(matches!(vars.get("items"), Some(Value::Sequence(_))));
    }

    fn create_shell_task(name: &str, command: &str) -> Task {
        let mut task = create_command_task(name, command);
        task.module = "shell".to_string();
        task
    }

    #[test]
    fn rejects_zero_forks_with_a_diagnostic_error() {
        let play = create_test_play();
        let error = play.execute_with_options(&[], false, 0).unwrap_err();
        assert!(error.to_string().contains("forks"));
        assert!(error.to_string().contains("greater than zero"));
    }

    #[test]
    fn idle_slots_take_later_hosts_before_slow_host_finishes_and_keep_task_barrier() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_str().unwrap();
        let script = temp.path().join("work.sh");
        fs::create_dir(temp.path().join("active")).unwrap();
        fs::write(
            &script,
            format!(
                r#"#!/bin/sh
set -eu
root='{root}'
mkdir "$root/active/$1"
trap 'rmdir "$root/active/$1"' EXIT
set -- "$1" "$root"/active/*
[ "$#" -le 3 ] || {{ touch "$root/overflow"; exit 43; }}
case "$1" in
slow)
    for attempt in $(seq 1 200); do
        [ ! -f "$root/release" ] || exit 0
        sleep 0.05
    done
    exit 42;;
last) touch "$root/release";;
esac
"#
            ),
        )
        .unwrap();
        let hosts = ["slow", "fast", "last"]
            .map(|name| create_test_host(name, "localhost", 22, None, None));
        let mut play = create_test_play();
        // Warm each lease before exercising scheduling. Copying debug workers
        // under parallel test IO can exceed the business latch's timeout.
        play.gather_facts = true;
        let mut work = create_command_task(
            "work",
            &format!("/bin/sh {} {{{{ inventory_hostname }}}}", script.display()),
        );
        work.register = Some("work_result".into());
        play.tasks.push(work);
        play.tasks.push(create_command_task(
            "barrier",
            &format!("/bin/sh -c 'test -z \"$(ls -A {root}/active)\"'"),
        ));
        let mut state = RunState::default();
        play.execute_in_run(&hosts, false, 2, &mut state).unwrap();
        assert!(temp.path().join("release").exists());
        assert!(!temp.path().join("overflow").exists());
        for host in &hosts {
            let shared = state.host_state().unwrap();
            let result = &shared.contexts[&host.name].registered["work_result"];
            assert_eq!(result["rc"].as_i64(), Some(0));
            assert_eq!(state.stats[&host.name].ok, 3);
        }
    }

    #[test]
    fn rejects_duplicate_host_names_before_building_play_state() {
        let play = create_test_play();
        let hosts = vec![create_local_host(), create_local_host()];
        let error = play.execute(&hosts).unwrap_err();
        assert!(error.to_string().contains("duplicate inventory host"));
        assert!(error.to_string().contains("localhost"));
    }

    #[test]
    fn test_play_execute_sequential() {
        let mut play = create_test_play();
        play.tasks.push(create_command_task("Task 1", "echo task1"));
        play.tasks.push(create_command_task("Task 2", "echo task2"));

        let hosts = vec![create_local_host()];
        let result = play.execute(&hosts);

        assert!(result.is_ok());
    }

    #[test]
    fn test_play_execute_failure() {
        let mut play = create_test_play();
        // False command fails with exit code 1
        play.tasks.push(create_command_task("Fail Task", "false"));

        let hosts = vec![create_local_host()];
        let result = play.execute(&hosts);

        // Should return error because task failed
        assert!(result.is_err());
    }

    #[test]
    fn test_play_execute_ignore_errors() {
        let mut play = create_test_play();
        let mut task = create_command_task("Fail Task", "false");
        task.ignore_errors = Some(true);
        play.tasks.push(task);
        play.tasks
            .push(create_command_task("Success Task", "echo success"));

        let hosts = vec![create_local_host()];
        let result = play.execute(&hosts);

        // Should succeed because ignore_errors is true
        assert!(result.is_ok());
    }

    #[test]
    fn test_play_execute_when_condition() {
        let mut play = create_test_play();
        let mut task = create_command_task("Conditional Task", "echo ran");
        // Condition that evaluates to false
        task.when = Some(Value::String("1 == 2".to_string()));
        play.tasks.push(task);

        let hosts = vec![create_local_host()];
        let result = play.execute(&hosts);

        assert!(result.is_ok());
        // In a real test we'd check if it ran, but here we just check it didn't crash
        // and returned success (skipped tasks are successful)
    }

    #[test]
    fn task_can_explicitly_disable_play_become() {
        let mut play = create_test_play();
        play.become_override = Some(true);
        play.is_become = true;
        play.become_user_override = Some("admin".to_string());
        play.become_user = "admin".to_string();

        let inherited = create_command_task("Inherited", "true");
        let host = create_local_host();
        let inherited = play.apply_play_settings(&inherited, &host);
        assert!(inherited.is_become);
        assert_eq!(inherited.become_user, "admin");

        let mut disabled = create_command_task("Disabled", "true");
        disabled.become_override = Some(false);
        let disabled = play.apply_play_settings(&disabled, &host);
        assert!(!disabled.is_become);
    }

    #[test]
    fn become_settings_follow_task_play_inventory_precedence() {
        let mut host = create_local_host();
        host.set_variable("ansible_become", "true");
        host.set_variable("ansible_become_user", "inventory-user");
        let mut play = create_test_play();
        let task = create_command_task("Inherited", "true");

        let inherited = play.apply_play_settings(&task, &host);
        assert!(inherited.is_become);
        assert_eq!(inherited.become_user, "inventory-user");

        play.become_override = Some(false);
        play.become_user_override = Some("root".to_string());
        let play_override = play.apply_play_settings(&task, &host);
        assert!(!play_override.is_become);
        assert_eq!(play_override.become_user, "root");

        let mut task_override = task;
        task_override.become_override = Some(true);
        task_override.become_user_override = Some("task-user".to_string());
        let task_override = play.apply_play_settings(&task_override, &host);
        assert!(task_override.is_become);
        assert_eq!(task_override.become_user, "task-user");
    }

    #[test]
    fn lexical_become_settings_survive_play_defaults_and_child_false() {
        let host = create_local_host();
        let mut play = create_test_play();
        play.become_override = Some(true);
        play.become_user_override = Some("root".into());
        let mut parent = create_command_task("Block", "true");
        parent.become_override = Some(true);
        parent.become_user_override = Some("app".into());
        let mut child = create_command_task("Child", "true");
        child.inherit(&parent);
        let inherited = play.apply_play_settings(&child, &host);
        assert_eq!(inherited.become_user_override.as_deref(), Some("app"));
        assert_eq!(inherited.become_user, "app");
        assert!(inherited.is_become);
        child.become_override = Some(false);
        let disabled = play.apply_play_settings(&child, &host);
        assert!(!disabled.is_become);
        assert_eq!(disabled.become_override, Some(false));
    }

    #[test]
    fn test_registered_result_persists_between_tasks() {
        let mut play = create_test_play();
        let mut capture = create_command_task("Capture", "printf registered");
        capture.register = Some("captured".to_string());
        play.tasks.push(capture);
        play.tasks.push(create_command_task(
            "Use registered value",
            "test \"{{ captured.stdout }}\" = registered",
        ));

        assert!(play.execute(&[create_local_host()]).is_ok());
    }

    #[test]
    fn test_handler_notification_is_scoped_to_triggering_host() {
        let directory = tempdir().unwrap();
        let output = directory.path().join("handler-hosts");
        let mut play = create_test_play();
        let mut trigger = create_command_task("Trigger", "true");
        trigger.when = Some(Value::String("inventory_hostname == 'first'".to_string()));
        trigger.notify.push("record handler".to_string());
        play.tasks.push(trigger);
        play.handlers.push(Handler {
            listen: Vec::new(),
            hosts: None,
            cached_name: None,
            task: create_shell_task(
                "record handler",
                &format!(
                    "printf '%s' '{{{{ inventory_hostname }}}}' >> {}",
                    output.display()
                ),
            ),
        });
        let hosts = vec![
            create_test_host("first", "localhost", 22, None, None),
            create_test_host("second", "localhost", 22, None, None),
        ];

        assert!(play.execute(&hosts).is_ok());
        assert_eq!(fs::read_to_string(output).unwrap(), "first");
    }

    #[test]
    fn test_failed_host_is_removed_but_healthy_hosts_continue() {
        let directory = tempdir().unwrap();
        let output = directory.path().join("continued-hosts");
        let mut play = create_test_play();
        let mut fail_first = create_command_task("Fail first", "false");
        fail_first.when = Some(Value::String("inventory_hostname == 'first'".to_string()));
        play.tasks.push(fail_first);
        play.tasks.push(create_shell_task(
            "Record survivors",
            &format!(
                "printf '%s' '{{{{ inventory_hostname }}}}' >> {}",
                output.display()
            ),
        ));
        let hosts = vec![
            create_test_host("first", "localhost", 22, None, None),
            create_test_host("second", "localhost", 22, None, None),
        ];

        assert!(play.execute(&hosts).is_err());
        assert_eq!(fs::read_to_string(output).unwrap(), "second");
    }

    #[test]
    fn play_stops_scheduling_when_every_host_has_failed() {
        let directory = tempdir().unwrap();
        let marker = directory.path().join("must-not-run");
        let mut play = create_test_play();
        play.tasks.push(create_command_task("Fail", "false"));
        play.tasks.push(create_command_task(
            "No active hosts",
            &format!("touch {}", marker.display()),
        ));

        assert!(play.execute(&[create_local_host()]).is_err());
        assert!(!marker.exists());
    }
}
