use anyhow::{bail, Context, Result};
use log::debug;
use serde::Deserialize;
use serde_yaml::{Mapping, Value};
use std::io::Read;
use std::path::Path;

use crate::playbook::{Handler, Play, Task};

/// The main Playbook structure
#[derive(Debug)]
pub struct Playbook {
    pub plays: Vec<Play>,
}

#[derive(Clone, Default)]
pub(crate) struct ParseOptions {
    pub variables: std::collections::HashMap<String, Value>,
    pub scope: crate::playbook::TaskScope,
    pub playbook_dir: Option<std::path::PathBuf>,
    pub role_paths: Vec<std::path::PathBuf>,
}

#[derive(Default)]
pub(crate) struct Expansion {
    pub tasks: Vec<Task>,
    pub handlers: Vec<Handler>,
    pub defaults: Mapping,
    pub variables: Mapping,
    scope_defaults: Mapping,
    scope_vars: Mapping,
}
impl Expansion {
    fn extend(&mut self, other: Self) {
        self.tasks.extend(other.tasks);
        self.handlers.extend(other.handlers);
        crate::vars::merge(&mut self.defaults, other.defaults);
        crate::vars::merge(&mut self.variables, other.variables);
        crate::vars::merge(&mut self.scope_defaults, other.scope_defaults);
        crate::vars::merge(&mut self.scope_vars, other.scope_vars);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RoleStyle {
    Play,
    Dependency,
    Import,
    Include,
}
struct RoleSpec {
    args: Mapping,
    settings: Task,
    params: Mapping,
    style: RoleStyle,
}

fn role_entry(value: &Value) -> Result<RoleSpec> {
    let mapping = match value {
        Value::String(name) => {
            Mapping::from_iter([(Value::String("role".into()), Value::String(name.clone()))])
        }
        Value::Mapping(mapping) => mapping.clone(),
        _ => bail!("Role declaration must be a name or mapping"),
    };
    let name = mapping
        .get(Value::String("role".into()))
        .or_else(|| mapping.get(Value::String("name".into())))
        .context("Role declaration requires role/name")?
        .clone();
    let mut args = Mapping::from_iter([(Value::String("name".into()), name)]);
    let mut attrs = Mapping::new();
    let mut params = Mapping::new();
    for (key, value) in &mapping {
        let name = key.as_str().context("Role keys must be strings")?;
        if matches!(name, "role" | "name") {
            continue;
        }
        if matches!(
            name,
            "tasks_from"
                | "vars_from"
                | "defaults_from"
                | "handlers_from"
                | "public"
                | "allow_duplicates"
        ) {
            args.insert(key.clone(), value.clone());
        } else if SUPPORTED_TASK_KEYWORDS.contains(&name) {
            attrs.insert(key.clone(), value.clone());
        } else {
            crate::vars::validate_name(name, "Role parameters")?;
            params.insert(key.clone(), value.clone());
        }
    }
    let settings = parse_task_parts(attrs, TaskKind::Task, "role".into(), Mapping::new())?;
    if settings.loop_items.is_some()
        || settings.register.is_some()
        || settings.until.is_some()
        || settings.retries.is_some()
    {
        bail!("Role declarations cannot loop/register/retry; use include_role where applicable");
    }
    Ok(RoleSpec {
        args,
        settings,
        params,
        style: RoleStyle::Play,
    })
}

fn role_variables(path: &Path, directory: &str, name: &str) -> Result<Mapping> {
    let mut variables = Mapping::new();
    for file in crate::vars::named_files(&path.join(directory), name)? {
        crate::vars::merge(&mut variables, crate::vars::load(&file)?);
    }
    Ok(variables)
}

fn apply_settings(args: &Mapping) -> Result<Option<Task>> {
    let Some(value) = args.get(Value::String("apply".into())) else {
        return Ok(None);
    };
    let mapping = value
        .as_mapping()
        .context("Include apply must be a mapping")?
        .clone();
    for key in mapping.keys().filter_map(Value::as_str) {
        if !SUPPORTED_TASK_KEYWORDS.contains(&key)
            || matches!(
                key,
                "async"
                    | "poll"
                    | "loop"
                    | "with_items"
                    | "loop_control"
                    | "register"
                    | "until"
                    | "retries"
                    | "delay"
            )
        {
            bail!("Unsupported include apply keyword: {key}");
        }
    }
    Ok(Some(parse_task_parts(
        mapping,
        TaskKind::Task,
        "include apply".into(),
        Mapping::new(),
    )?))
}

fn expand_role(
    request: &crate::playbook::task::IncludeRequest,
    options: &ParseOptions,
) -> Result<Expansion> {
    let source = request
        .parent
        .source_file
        .as_ref()
        .context("Role include has no source")?;
    let raw = request.parent.raw();
    let mut settings = parse_task_parts(
        Mapping::new(),
        TaskKind::Task,
        "role".into(),
        Mapping::new(),
    )?;
    settings.vars = raw.vars.clone();
    settings.source_file = Some(source.clone());
    settings.scope = request.parent.scope.clone();
    if let Some(apply) = apply_settings(&request.args)? {
        settings.inherit(&apply);
    }
    if let Some(parent) = &raw.parent {
        settings.inherit(parent);
    }
    let mut options = options.clone();
    options.scope = request.parent.scope.clone();
    let mut loader = Loader {
        options: &options,
        stack: options.scope.ancestors.clone(),
        playbook_dir: options
            .playbook_dir
            .clone()
            .unwrap_or_else(|| source.parent().expect("Source parent").to_path_buf()),
        exports: Expansion::default(),
    };
    loader.role(
        RoleSpec {
            args: request.args.clone(),
            settings,
            params: Mapping::new(),
            style: RoleStyle::Include,
        },
        &Mapping::new(),
        true,
    )
}

impl Loader<'_> {
    fn find_role(&self, name: &str) -> Result<std::path::PathBuf> {
        let mut roots = vec![self.playbook_dir.join("roles")];
        if self.options.role_paths.is_empty() {
            if let Some(home) = std::env::var_os("HOME") {
                roots.push(std::path::PathBuf::from(home).join(".ansible/roles"));
            }
            roots.extend([
                "/usr/share/ansible/roles".into(),
                "/etc/ansible/roles".into(),
            ]);
        } else {
            roots.extend(self.options.role_paths.clone());
        }
        if let Some(parent) = self
            .options
            .scope
            .role_path
            .as_ref()
            .and_then(|path| path.parent())
        {
            roots.push(parent.to_path_buf());
        }
        roots.push(self.playbook_dir.clone());
        roots.push(std::env::current_dir()?);
        for root in &roots {
            let path = root.join(name);
            if path.is_dir() {
                return Ok(path.canonicalize()?);
            }
        }
        bail!(
            "Role '{name}' was not found in {}",
            roots
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(":")
        );
    }
    fn role(
        &mut self,
        spec: RoleSpec,
        variables: &Mapping,
        resolved_name: bool,
    ) -> Result<Expansion> {
        let allowed = [
            "name",
            "tasks_from",
            "vars_from",
            "defaults_from",
            "handlers_from",
            "public",
            "allow_duplicates",
            "apply",
        ];
        crate::modules::param::validate_params(&Value::Mapping(spec.args.clone()), &allowed)?;
        let name = spec
            .args
            .get(Value::String("name".into()))
            .context("Role requires name")?;
        let name = if resolved_name {
            name.as_str()
                .context("Role name must be a string")?
                .to_owned()
        } else {
            self.render_string(name, variables)?
        };
        if name.is_empty() || name.contains('\0') {
            bail!("Role name must be nonempty without NUL");
        }
        let path = self.find_role(&name)?;
        let mut chain = self.options.scope.parent_roles.clone();
        if let Some(parent) = &self.options.scope.role_path {
            chain.push(parent.clone());
        }
        if chain.contains(&path) || chain.len() >= 64 {
            chain.push(path);
            bail!(
                "Role dependency/include cycle: {}",
                chain
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(" -> ")
            );
        }
        for directory in [
            "library",
            "module_utils",
            "action_plugins",
            "filter_plugins",
            "lookup_plugins",
            "inventory_plugins",
            "connection_plugins",
            "become_plugins",
            "callback_plugins",
            "strategy_plugins",
            "vars_plugins",
            "test_plugins",
        ] {
            let directory = path.join(directory);
            if directory.is_dir() && std::fs::read_dir(&directory)?.next().is_some() {
                bail!(
                    "Role plugin content is unsupported and is never executed automatically: {}",
                    directory.display()
                );
            }
        }
        if !crate::vars::named_files(&path.join("meta"), "argument_specs")?.is_empty() {
            bail!(
                "Role argument_specs validation is not yet supported: {}",
                path.display()
            );
        }
        let file_name = |key: &str| -> Result<&str> {
            match spec.args.get(Value::String(key.into())) {
                None => Ok("main"),
                Some(Value::String(name)) if !name.is_empty() && !name.contains('\0') => Ok(name),
                _ => bail!("Role {key} must be a nonempty filename string"),
            }
        };
        let defaults = role_variables(&path, "defaults", file_name("defaults_from")?)?;
        let role_vars = role_variables(&path, "vars", file_name("vars_from")?)?;
        let metadata = role_variables(&path, "meta", "main")?;
        let meta_duplicate =
            parse_optional_bool(&metadata, "allow_duplicates", "Role meta")?.unwrap_or(false);
        let duplicate = parse_optional_bool(&spec.args, "allow_duplicates", "Role")?.unwrap_or(
            if matches!(spec.style, RoleStyle::Play | RoleStyle::Dependency) {
                meta_duplicate
            } else {
                true
            },
        );
        let public = spec.style != RoleStyle::Dependency
            && parse_optional_bool(&spec.args, "public", "Role")?
                .unwrap_or(spec.style != RoleStyle::Include);
        let mut options = self.options.clone();
        crate::vars::merge(&mut options.scope.defaults, defaults.clone());
        crate::vars::merge(&mut options.scope.role_vars, role_vars.clone());
        crate::vars::merge(&mut options.scope.role_vars, spec.settings.vars.clone());
        crate::vars::merge(&mut options.scope.role_params, spec.params.clone());
        if spec.style == RoleStyle::Include {
            crate::vars::merge(&mut options.scope.params, spec.settings.vars.clone());
        }
        options.scope.parent_roles = chain;
        options.scope.role_path = Some(path.clone());
        options.scope.role_name = Some(
            path.file_name()
                .context("Role has no name")?
                .to_string_lossy()
                .into_owned(),
        );
        options.scope.allow_duplicates = duplicate;
        let mut settings = spec.settings.effective();
        let identity = serde_json::to_vec(&serde_json::to_value((
            &path,
            (
                file_name("tasks_from")?,
                file_name("vars_from")?,
                file_name("defaults_from")?,
                file_name("handlers_from")?,
            ),
            &options.scope.role_params,
            &settings.vars,
            &settings.when,
            &settings.tags,
            (
                &settings.become_override,
                &settings.become_user_override,
                &settings.connection,
                &settings.environment,
                &settings.check_mode,
                &settings.ignore_errors,
                &settings.no_log,
            ),
        ))?)?;
        let id = rustsible_protocol::sha256(&mut identity.as_slice())?;
        options.scope.role_id = Some(id);
        options.scope.boundary = None;
        settings.scope = options.scope.clone();
        settings.original = None;
        settings.parent = None;
        let mut known = variables.clone();
        crate::vars::merge(&mut known, role_vars.clone());
        crate::vars::merge(&mut known, settings.vars.clone());
        let mut loader = Loader {
            options: &options,
            stack: self.stack.clone(),
            playbook_dir: self.playbook_dir.clone(),
            exports: Expansion::default(),
        };
        let mut expansion = Expansion::default();
        if let Some(dependencies) = metadata.get(Value::String("dependencies".into())) {
            let dependencies = dependencies
                .as_sequence()
                .context("Role meta dependencies must be a list")?;
            for dependency in dependencies {
                let mut dependency = role_entry(dependency)?;
                dependency.style = RoleStyle::Dependency;
                let mut parent = settings.clone();
                parent.vars.clear();
                dependency.settings.inherit(&parent);
                expansion.extend(loader.role(dependency, &known, false)?);
            }
        }
        drop(loader);
        options.scope.defaults = self.options.scope.defaults.clone();
        crate::vars::merge(
            &mut options.scope.defaults,
            expansion.scope_defaults.clone(),
        );
        crate::vars::merge(&mut options.scope.defaults, defaults.clone());
        options.scope.role_vars = self.options.scope.role_vars.clone();
        crate::vars::merge(&mut options.scope.role_vars, expansion.scope_vars.clone());
        crate::vars::merge(&mut options.scope.role_vars, role_vars.clone());
        crate::vars::merge(&mut options.scope.role_vars, settings.vars.clone());
        settings.scope = options.scope.clone();
        let mut loader = Loader {
            options: &options,
            stack: self.stack.clone(),
            playbook_dir: self.playbook_dir.clone(),
            exports: Expansion::default(),
        };
        let mut body = Vec::new();
        for file in crate::vars::named_files(&path.join("tasks"), file_name("tasks_from")?)? {
            body.extend(
                loader
                    .task_file(&file, TaskKind::Task, &known)?
                    .into_iter()
                    .map(|entry| entry.task),
            );
        }
        for task in &mut body {
            task.inherit(&settings);
        }
        let mut handlers = Vec::new();
        for file in crate::vars::named_files(&path.join("handlers"), file_name("handlers_from")?)? {
            handlers.extend(loader.task_file(&file, TaskKind::Handler, &known)?);
        }
        for handler in &mut handlers {
            handler.task.inherit(&settings);
        }
        let marker = |boundary| {
            let mut task = settings.clone();
            task.module = "meta".into();
            task.args = Mapping::new();
            task.when = None;
            task.loop_items = None;
            task.scope.boundary = Some(boundary);
            task
        };
        let mut tasks = vec![marker(crate::playbook::task::Boundary::Start)];
        tasks.extend(expansion.tasks);
        tasks.extend(body);
        tasks.push(marker(crate::playbook::task::Boundary::End));
        expansion.tasks = tasks;
        expansion.handlers.extend(handlers);
        expansion.extend(std::mem::take(&mut loader.exports));
        crate::vars::merge(&mut expansion.scope_defaults, defaults);
        crate::vars::merge(&mut expansion.scope_vars, role_vars);
        if public {
            let mut defaults = expansion.scope_defaults.clone();
            crate::vars::merge(&mut defaults, expansion.defaults);
            expansion.defaults = defaults;
            let mut variables = expansion.scope_vars.clone();
            crate::vars::merge(&mut variables, expansion.variables);
            expansion.variables = variables;
        }
        Ok(expansion)
    }
    fn collect_exports(&mut self, expanded: &mut Expansion) {
        self.exports.handlers.append(&mut expanded.handlers);
        crate::vars::merge(
            &mut self.exports.defaults,
            std::mem::take(&mut expanded.defaults),
        );
        crate::vars::merge(
            &mut self.exports.variables,
            std::mem::take(&mut expanded.variables),
        );
    }
}

struct Loader<'a> {
    options: &'a ParseOptions,
    stack: Vec<std::path::PathBuf>,
    playbook_dir: std::path::PathBuf,
    exports: Expansion,
}

/// Static imports are expanded before a host can be contacted. Each leaf keeps
/// its real source path; the stack guards ancestry, not repeated sibling use.
pub fn parse_playbook(playbook_path: &str, options: &ParseOptions) -> Result<Playbook> {
    let path = Path::new(playbook_path)
        .canonicalize()
        .context("Cannot resolve playbook source path")?;
    let mut loader = Loader {
        exports: Expansion::default(),
        options,
        stack: Vec::new(),
        playbook_dir: path
            .parent()
            .context("Playbook has no parent")?
            .to_path_buf(),
    };
    Ok(Playbook {
        plays: loader.playbook(&path, &Mapping::new())?,
    })
}

pub(crate) fn expand_tasks(
    request: &crate::playbook::task::IncludeRequest,
    options: &ParseOptions,
) -> Result<Expansion> {
    if request.kind == crate::playbook::task::IncludeKind::Role {
        return expand_role(request, options);
    }
    let source = request
        .parent
        .source_file
        .as_ref()
        .context("Dynamic include has no source file")?;
    let file = request
        .args
        .get(Value::String("file".into()))
        .and_then(Value::as_str)
        .context("include_tasks requires file")?;
    let path = source
        .parent()
        .context("Include has no parent directory")?
        .join(file);
    let mut options = options.clone();
    options.scope = request.parent.scope.clone();
    let mut variables = Mapping::new();
    crate::vars::merge(&mut variables, request.parent.scope.role_vars.clone());
    crate::vars::merge(&mut variables, request.parent.scope.params.clone());
    crate::vars::merge(&mut variables, request.parent.vars.clone());
    let mut loader = Loader {
        exports: Expansion::default(),
        stack: options.scope.ancestors.clone(),
        playbook_dir: options
            .playbook_dir
            .clone()
            .unwrap_or_else(|| source.parent().expect("Source parent").to_path_buf()),
        options: &options,
    };
    let mut tasks = loader
        .task_file(&path, TaskKind::Task, &variables)?
        .into_iter()
        .map(|entry| entry.task)
        .collect::<Vec<_>>();
    let settings = apply_settings(&request.args)?;
    for task in &mut tasks {
        if let Some(settings) = &settings {
            task.inherit(settings);
        }
        if let Some(ancestor) = &request.parent.raw().parent {
            task.inherit(ancestor);
        }
        let mut params = request.parent.scope.params.clone();
        crate::vars::merge(&mut params, request.parent.raw().vars.clone());
        crate::vars::merge(&mut params, task.scope.params.clone());
        task.scope.params = params;
        let mut locals = request.parent.scope.locals.clone();
        locals.extend(task.scope.locals.clone());
        task.scope.locals = locals;
    }
    let mut expanded = std::mem::take(&mut loader.exports);
    expanded.tasks = tasks;
    Ok(expanded)
}

impl Loader<'_> {
    fn enter(&mut self, path: &Path) -> Result<std::path::PathBuf> {
        let path = path
            .canonicalize()
            .with_context(|| format!("Cannot resolve imported file {}", path.display()))?;
        if self.stack.contains(&path) || self.stack.len() >= 64 {
            let mut chain = self
                .stack
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>();
            chain.push(path.display().to_string());
            bail!("Import cycle or depth limit: {}", chain.join(" -> "));
        }
        self.stack.push(path.clone());
        Ok(path)
    }
    fn playbook(&mut self, path: &Path, bindings: &Mapping) -> Result<Vec<Play>> {
        let source = self.enter(path)?;
        let previous_dir = std::mem::replace(
            &mut self.playbook_dir,
            source
                .parent()
                .context("Playbook has no parent")?
                .to_path_buf(),
        );
        let result = (|| {
            let content = read_file(&source)?;
            let mut plays = Vec::new();
            for (index, document) in serde_yaml::Deserializer::from_str(&content).enumerate() {
                let value = Value::deserialize(document).with_context(|| {
                    format!(
                        "Failed to parse YAML document {} ({})",
                        index + 1,
                        source.display()
                    )
                })?;
                let entries = match value {
                    Value::Sequence(entries) if !entries.is_empty() => entries,
                    Value::Mapping(mapping) => vec![Value::Mapping(mapping)],
                    _ => bail!(
                        "YAML document {} must contain at least one play mapping",
                        index + 1
                    ),
                };
                for entry in entries {
                    let mut mapping = entry
                        .as_mapping()
                        .context("Play must be a mapping")?
                        .clone();
                    if let Some((key, _)) = mapping.iter().find(|(key, _)| {
                        key.as_str()
                            .is_some_and(|name| short_name(name) == "import_playbook")
                    }) {
                        let key = key.clone();
                        for key in mapping.keys().filter_map(Value::as_str) {
                            if !matches!(
                                short_name(key),
                                "import_playbook" | "vars" | "name" | "when"
                            ) {
                                bail!("Unsupported import_playbook keyword: {key}");
                            }
                        }
                        let mut variables = parse_variables(
                            mapping.get(Value::String("vars".into())),
                            "import_playbook vars",
                        )?;
                        crate::vars::merge(&mut variables, bindings.clone());
                        let file = self.filename(
                            mapping.get(&key).context("Missing import_playbook file")?,
                            &variables,
                            &source,
                        )?;
                        let mut imported = self.playbook(&file, &variables)?;
                        if let Some(condition) = mapping.get(Value::String("when".into())) {
                            validate_condition(condition, "import_playbook when")?;
                            let mut parent = flush_task();
                            parent.scope.boundary = None;
                            parent.when = Some(condition.clone());
                            for play in &mut imported {
                                for task in play
                                    .pre_tasks
                                    .iter_mut()
                                    .chain(&mut play.tasks)
                                    .chain(&mut play.post_tasks)
                                {
                                    task.inherit(&parent);
                                }
                                let mut conditions = vec![condition.clone()];
                                if let Some(inherited) = play.import_condition.take() {
                                    if let Value::Sequence(values) = inherited {
                                        conditions.extend(values);
                                    } else {
                                        conditions.push(inherited);
                                    }
                                }
                                // Gathering must be gated too, before opening a target connection.
                                play.import_condition = Some(Value::Sequence(conditions));
                            }
                        }
                        plays.extend(imported);
                    } else {
                        let mut variables = parse_variables(
                            mapping.get(Value::String("vars".into())),
                            "Play vars",
                        )?;
                        crate::vars::merge(&mut variables, bindings.clone());
                        mapping.insert(Value::String("vars".into()), Value::Mapping(variables));
                        let mut play = self
                            .parse_play(mapping, plays.len() + 1, &source)
                            .with_context(|| {
                                format!(
                                    "Failed to parse play {} in document {} ({})",
                                    plays.len() + 1,
                                    index + 1,
                                    source.display()
                                )
                            })?;
                        play.source_file = Some(source.clone());
                        plays.push(play);
                    }
                }
            }
            if plays.is_empty() {
                bail!("Playbook contains no plays");
            }
            Ok(plays)
        })();
        self.playbook_dir = previous_dir;
        self.stack.pop();
        result
    }
    fn render_string(&self, value: &Value, variables: &Mapping) -> Result<String> {
        let text = value.as_str().context("Import filename must be a string")?;
        let mut declared = self.options.scope.defaults.clone();
        crate::vars::merge(&mut declared, self.options.scope.role_vars.clone());
        crate::vars::merge(&mut declared, variables.clone());
        crate::vars::merge(&mut declared, self.options.scope.role_params.clone());
        crate::vars::merge(&mut declared, self.options.scope.params.clone());
        let mut definitions = declared
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().expect("Validated variable").to_owned(),
                    value.clone(),
                )
            })
            .collect::<std::collections::HashMap<_, _>>();
        crate::inventory::host::merge_variable_layer(&mut definitions, &self.options.variables);
        let data = std::collections::HashMap::from([(
            "playbook_dir".into(),
            Value::String(self.playbook_dir.to_string_lossy().into_owned()),
        )]);
        let mut data = data;
        data.extend(self.options.scope.locals.clone());
        if let Some(path) = &self.options.scope.role_path {
            data.insert(
                "role_path".into(),
                Value::String(path.to_string_lossy().into_owned()),
            );
        }
        let variables = crate::playbook::templar::resolve_definitions(&definitions, &data)?;
        let context = crate::playbook::templar::create_context(&variables)?;
        let value = crate::playbook::templar::render_value(text, &mut crate::playbook::templar::environment(), &context, true).context("Static import path cannot use unresolved host/runtime variables; use include_tasks instead")?;
        let path = value
            .as_str()
            .filter(|path| !path.is_empty() && !path.contains('\0'))
            .context("Import path must resolve to a nonempty string without NUL")?;
        Ok(path.to_owned())
    }
    fn filename(
        &self,
        value: &Value,
        variables: &Mapping,
        source: &Path,
    ) -> Result<std::path::PathBuf> {
        Ok(source
            .parent()
            .context("Import source has no parent")?
            .join(self.render_string(value, variables)?))
    }
    fn task_file(
        &mut self,
        path: &Path,
        kind: TaskKind,
        variables: &Mapping,
    ) -> Result<Vec<Handler>> {
        let source = self.enter(path)?;
        let result: Result<Vec<Handler>> = (|| {
            let content = read_file(&source)?;
            let mut tasks = Vec::new();
            for document in serde_yaml::Deserializer::from_str(&content) {
                let value = Value::deserialize(document)
                    .with_context(|| format!("Invalid imported task file {}", source.display()))?;
                if value.is_null() {
                    continue;
                }
                tasks.extend(self.task_list(Some(&value), kind, &source, variables)?);
            }
            Ok(tasks)
        })();
        self.stack.pop();
        result.with_context(|| format!("Imported task file {}", source.display()))
    }
}

fn read_file(path: &Path) -> Result<String> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .with_context(|| format!("Cannot open source file {}", path.display()))?;
    if !file.metadata()?.is_file() {
        bail!("Source must be a regular file: {}", path.display());
    }
    let mut content = String::new();
    file.read_to_string(&mut content)?;
    Ok(content)
}
pub(crate) fn flush_task() -> Task {
    let mut task = parse_task_parts(
        Mapping::new(),
        TaskKind::Task,
        "meta".into(),
        Mapping::from_iter([(
            Value::String("action".into()),
            Value::String("flush_handlers".into()),
        )]),
    )
    .expect("Synthetic flush has valid attributes");
    task.scope.boundary = Some(crate::playbook::task::Boundary::Flush);
    task
}

fn short_name(name: &str) -> &str {
    name.strip_prefix("ansible.builtin.")
        .or_else(|| name.strip_prefix("ansible.legacy."))
        .unwrap_or(name)
}

const SUPPORTED_PLAY_KEYWORDS: &[&str] = &[
    "strategy",
    "serial",
    "throttle",
    "run_once",
    "any_errors_fatal",
    "max_fail_percentage",
    "force_handlers",
    "ignore_errors",
    "ignore_unreachable",
    "no_log",
    "pre_tasks",
    "post_tasks",
    "roles",
    "vars_files",
    "name",
    "environment",
    "hosts",
    "tasks",
    "handlers",
    "vars",
    "become",
    "become_user",
    "connection",
    "fail_fast",
    "gather_facts",
    "gather_subset",
    "gather_timeout",
];

const UNSUPPORTED_PLAY_KEYWORDS: &[&str] = &[
    "become_exe",
    "become_flags",
    "become_method",
    "check_mode",
    "collections",
    "debugger",
    "diff",
    "fact_path",
    "module_defaults",
    "order",
    "port",
    "remote_user",
    "tags",
    "timeout",
    "vars_prompt",
];

const SUPPORTED_TASK_KEYWORDS: &[&str] = &[
    "async",
    "poll",
    "delegate_to",
    "delegate_facts",
    "throttle",
    "run_once",
    "any_errors_fatal",
    "ignore_unreachable",
    "name",
    "args",
    "environment",
    "timeout",
    "delay",
    "retries",
    "until",
    "failed_when",
    "changed_when",
    "become",
    "become_user",
    "check_mode",
    "connection",
    "register",
    "when",
    "notify",
    "ignore_errors",
    "no_log",
    "vars",
    "with_items",
    "loop",
    "loop_control",
];

const UNSUPPORTED_TASK_KEYWORDS: &[&str] = &[
    "action",
    "always",
    "become_exe",
    "become_flags",
    "become_method",
    "block",
    "collections",
    "debugger",
    "diff",
    "listen",
    "module_defaults",
    "port",
    "remote_user",
    "rescue",
    "tags",
];

#[derive(Clone, Copy)]
enum TaskKind {
    Task,
    Handler,
}

impl TaskKind {
    fn label(self) -> &'static str {
        match self {
            Self::Task => "Task",
            Self::Handler => "Handler",
        }
    }
}

impl Loader<'_> {
    /// Parse an individual play from a YAML mapping.
    fn parse_play(&mut self, play_map: Mapping, ordinal: usize, source: &Path) -> Result<Play> {
        debug!("Parsing play definition");
        validate_play_keys(&play_map)?;
        let previous_exports = std::mem::take(&mut self.exports);

        let name = match play_map.get(Value::String("name".to_string())) {
            Some(Value::String(name)) if !name.trim().is_empty() => name.clone(),
            Some(Value::String(_)) => bail!("Play name cannot be empty"),
            Some(_) => bail!("Play name must be a string"),
            None => format!("Play {}", ordinal),
        };

        let hosts = match play_map.get(Value::String("hosts".to_string())) {
            Some(Value::String(hosts)) if !hosts.trim().is_empty() => hosts.clone(),
            Some(Value::String(_)) => bail!("Play hosts cannot be empty"),
            Some(_) => bail!("Hosts must be a string"),
            None => bail!("Play requires a hosts field"),
        };

        let vars = parse_variables(play_map.get(Value::String("vars".to_string())), "Play vars")?;
        let mut role_tasks = Vec::new();
        if let Some(roles) = play_map.get(Value::String("roles".into())) {
            for role in roles.as_sequence().context("Play roles must be a list")? {
                let mut expanded = self.role(role_entry(role)?, &vars, false)?;
                role_tasks.append(&mut expanded.tasks);
                self.collect_exports(&mut expanded);
            }
        }
        let mut tasks: Vec<Task> = self
            .task_list(
                play_map.get(Value::String("tasks".to_string())),
                TaskKind::Task,
                source,
                &vars,
            )?
            .into_iter()
            .map(|task| task.task)
            .collect();
        let mut handlers = self.task_list(
            play_map.get(Value::String("handlers".to_string())),
            TaskKind::Handler,
            source,
            &vars,
        )?;

        role_tasks.append(&mut tasks);
        tasks = role_tasks;
        let pre_tasks = self
            .task_list(
                play_map.get(Value::String("pre_tasks".into())),
                TaskKind::Task,
                source,
                &vars,
            )?
            .into_iter()
            .map(|entry| entry.task)
            .collect();
        let post_tasks = self
            .task_list(
                play_map.get(Value::String("post_tasks".into())),
                TaskKind::Task,
                source,
                &vars,
            )?
            .into_iter()
            .map(|entry| entry.task)
            .collect();
        let exported = std::mem::replace(&mut self.exports, previous_exports);
        let mut all_handlers = exported.handlers;
        all_handlers.append(&mut handlers);
        handlers = all_handlers;
        let vars_files = match play_map.get(Value::String("vars_files".into())) {
            None => Vec::new(),
            Some(Value::String(path)) => vec![Value::String(path.clone())],
            Some(Value::Sequence(paths)) => paths.clone(),
            _ => bail!("Play vars_files must be a string or list"),
        };
        for entry in &vars_files {
            let paths = entry
                .as_sequence()
                .map(Vec::as_slice)
                .unwrap_or(std::slice::from_ref(entry));
            if paths.is_empty()
                || paths
                    .iter()
                    .any(|path| path.as_str().is_none_or(str::is_empty))
            {
                bail!("vars_files entries must be nonempty paths or fallback lists");
            }
        }
        let become_override = parse_optional_bool(&play_map, "become", "Play")?;
        let is_become = become_override.unwrap_or(false);
        let become_user_override =
            parse_optional_nonempty_string(&play_map, "become_user", "Play")?;
        let become_user = become_user_override
            .clone()
            .unwrap_or_else(|| "root".to_string());
        let connection = parse_optional_nonempty_string(&play_map, "connection", "Play")?;
        let fail_fast = parse_optional_bool(&play_map, "fail_fast", "Play")?.unwrap_or(false);
        let gather_facts = parse_optional_bool(&play_map, "gather_facts", "Play")?.unwrap_or(true);
        let gather_subset = play_map.get(Value::String("gather_subset".into())).cloned();
        let gather_timeout = play_map
            .get(Value::String("gather_timeout".into()))
            .cloned();
        if gather_subset.as_ref().is_some_and(|value| {
            !value.is_string()
                && !value
                    .as_sequence()
                    .is_some_and(|values| values.iter().all(Value::is_string))
        }) {
            bail!("Play gather_subset must be a string or list of strings");
        }
        if gather_timeout
            .as_ref()
            .is_some_and(|value| !value.is_string() && !value.is_number())
        {
            bail!("Play gather_timeout must be an integer or template");
        }

        debug!(
            "Finished parsing play '{}' with {} tasks and {} handlers",
            name,
            tasks.len(),
            handlers.len()
        );

        let strategy = match parse_optional_nonempty_string(&play_map, "strategy", "Play")?
            .as_deref()
            .unwrap_or("linear")
        {
            "linear" | "ansible.builtin.linear" | "ansible.legacy.linear" => {
                crate::playbook::Strategy::Linear
            }
            "free" | "ansible.builtin.free" | "ansible.legacy.free" => {
                crate::playbook::Strategy::Free
            }
            strategy => bail!("Unsupported strategy: {strategy}"),
        };
        let play = Play {
            strategy,
            name,
            hosts,
            tasks,
            pre_tasks,
            post_tasks,
            handlers,
            vars,
            role_defaults: exported.defaults,
            role_vars: exported.variables,
            role_paths: self.options.role_paths.clone(),
            vars_files,
            source_file: None,
            environment: parse_environment(&play_map)?,
            become_override,
            is_become,
            become_user,
            become_user_override,
            fail_fast,
            serial: play_map.get(Value::String("serial".into())).cloned(),
            throttle: parse_optional_usize(&play_map, "throttle", "Play")?,
            run_once: parse_optional_bool(&play_map, "run_once", "Play")?,
            any_errors_fatal: parse_optional_bool(&play_map, "any_errors_fatal", "Play")?,
            max_fail_percentage: parse_optional_usize(&play_map, "max_fail_percentage", "Play")?
                .map(|value| {
                    if value > 100 {
                        bail!("max_fail_percentage must be between 0 and 100");
                    }
                    Ok(value as u8)
                })
                .transpose()?,
            force_handlers: parse_optional_bool(&play_map, "force_handlers", "Play")?
                .unwrap_or(false),
            ignore_errors: parse_optional_bool(&play_map, "ignore_errors", "Play")?,
            ignore_unreachable: parse_optional_bool(&play_map, "ignore_unreachable", "Play")?,
            no_log: parse_optional_bool(&play_map, "no_log", "Play")?,
            connection,
            gather_facts,
            import_condition: None,
            gather_subset,
            gather_timeout,
            tags: Vec::new(),
        };
        play.validate_strategy()?;
        Ok(play)
    }
}

fn validate_play_keys(play_map: &Mapping) -> Result<()> {
    for key in play_map.keys() {
        let Value::String(key) = key else {
            bail!("Play keys must be strings");
        };
        if SUPPORTED_PLAY_KEYWORDS.contains(&key.as_str()) {
            continue;
        }
        if UNSUPPORTED_PLAY_KEYWORDS.contains(&key.as_str()) {
            bail!("Play keyword '{}' is recognized but not supported", key);
        }
        bail!("Unknown play keyword '{}'", key);
    }
    Ok(())
}

impl Loader<'_> {
    fn task_list(
        &mut self,
        value: Option<&Value>,
        kind: TaskKind,
        source: &Path,
        variables: &Mapping,
    ) -> Result<Vec<Handler>> {
        let Some(value) = value else {
            return Ok(Vec::new());
        };
        let Value::Sequence(entries) = value else {
            bail!(
                "Play {}s must be a sequence of mappings",
                kind.label().to_lowercase()
            );
        };

        let mut parsed = Vec::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            let Value::Mapping(mapping) = entry else {
                bail!("{} {} must be a mapping", kind.label(), index + 1);
            };
            if mapping.contains_key(Value::String("block".into())) {
                if matches!(kind, TaskKind::Handler) {
                    bail!("Handlers cannot declare blocks");
                }
                let mut attrs = mapping.clone();
                for key in ["block", "rescue", "always"] {
                    attrs.remove(Value::String(key.into()));
                }
                for key in attrs.keys().filter_map(Value::as_str) {
                    if !SUPPORTED_TASK_KEYWORDS.contains(&key)
                        || matches!(
                            key,
                            "async"
                                | "poll"
                                | "loop"
                                | "with_items"
                                | "loop_control"
                                | "register"
                                | "until"
                                | "retries"
                                | "delay"
                        )
                    {
                        bail!("Unsupported block keyword: {key}");
                    }
                }
                let mut header =
                    parse_task_parts(attrs, TaskKind::Task, "block".into(), Mapping::new())?;
                header.source_file = Some(source.to_path_buf());
                header.scope = self.options.scope.clone();
                header.scope.ancestors = self.stack.clone();
                let mut tasks = self
                    .task_list(
                        mapping.get(Value::String("block".into())),
                        TaskKind::Task,
                        source,
                        variables,
                    )?
                    .into_iter()
                    .map(|entry| entry.task)
                    .collect::<Vec<_>>();
                let mut rescue = self
                    .task_list(
                        mapping.get(Value::String("rescue".into())),
                        TaskKind::Task,
                        source,
                        variables,
                    )?
                    .into_iter()
                    .map(|entry| entry.task)
                    .collect::<Vec<_>>();
                let mut always = self
                    .task_list(
                        mapping.get(Value::String("always".into())),
                        TaskKind::Task,
                        source,
                        variables,
                    )?
                    .into_iter()
                    .map(|entry| entry.task)
                    .collect::<Vec<_>>();
                for task in tasks.iter_mut().chain(&mut rescue).chain(&mut always) {
                    task.inherit(&header);
                }
                header.group = Some(Box::new(crate::playbook::TaskGroup {
                    tasks,
                    rescue,
                    always,
                }));
                parsed.push(Handler {
                    task: header,
                    listen: Vec::new(),
                    hosts: None,
                    cached_name: None,
                });
                continue;
            }
            if mapping
                .keys()
                .filter_map(Value::as_str)
                .any(|name| short_name(name) == "import_role")
            {
                if matches!(kind, TaskKind::Handler) {
                    bail!("import_role cannot be a handler");
                }
                let parent = parse_task(mapping.clone(), index, TaskKind::Task)?;
                if parent.loop_items.is_some() {
                    bail!("import_role does not support loops; use include_role");
                }
                let mut expanded = self.role(
                    RoleSpec {
                        args: parent.args.clone(),
                        settings: parent,
                        params: Mapping::new(),
                        style: RoleStyle::Import,
                    },
                    variables,
                    false,
                )?;
                let tasks = std::mem::take(&mut expanded.tasks);
                self.collect_exports(&mut expanded);
                parsed.extend(tasks.into_iter().map(|task| Handler {
                    task,
                    listen: Vec::new(),
                    hosts: None,
                    cached_name: None,
                }));
                continue;
            }
            if mapping
                .keys()
                .filter_map(Value::as_str)
                .any(|name| short_name(name) == "import_tasks")
            {
                let parent = parse_task(mapping.clone(), index, TaskKind::Task)?;
                if parent.loop_items.is_some() {
                    bail!("import_tasks does not support loops; use include_tasks");
                }
                crate::modules::param::validate_params(
                    &Value::Mapping(parent.args.clone()),
                    &["file"],
                )?;
                let mut scoped = variables.clone();
                crate::vars::merge(&mut scoped, parent.vars.clone());
                let file = self.filename(
                    parent
                        .args
                        .get(Value::String("file".into()))
                        .context("import_tasks requires file")?,
                    &scoped,
                    source,
                )?;
                let mut imported = self.task_file(&file, kind, &scoped)?;
                for entry in &mut imported {
                    entry.task.inherit(&parent);
                }
                parsed.extend(imported);
                continue;
            }
            let mut handler = match kind {
                TaskKind::Task => Handler {
                    listen: Vec::new(),
                    hosts: None,
                    cached_name: None,
                    task: parse_task(mapping.clone(), index, kind)?,
                },
                TaskKind::Handler => parse_handler(mapping.clone(), index)?,
            };
            handler.task.source_file = Some(source.to_path_buf());
            handler.task.scope = self.options.scope.clone();
            handler.task.scope.source_index = index;
            handler.task.scope.ancestors = self.stack.clone();
            parsed.push(handler);
        }
        Ok(parsed)
    }
}

/// Parse a task or handler from a YAML mapping.
fn parse_task(task_map: Mapping, index: usize, kind: TaskKind) -> Result<Task> {
    debug!("Parsing {} definition at index {}", kind.label(), index);

    let (module, args) = parse_module(&task_map, kind)?;
    if matches!(short_name(&module), "include_tasks" | "include_role") {
        // Includes select content; execution settings belong on their apply
        // block. Reject unsupported controls instead of accepting no-op retries.
        for key in task_map.keys().filter_map(Value::as_str) {
            if short_name(key) == short_name(&module) {
                continue;
            }
            if !matches!(
                key,
                "args"
                    | "ignore_errors"
                    | "loop"
                    | "with_items"
                    | "loop_control"
                    | "name"
                    | "no_log"
                    | "register"
                    | "run_once"
                    | "timeout"
                    | "vars"
                    | "when"
            ) {
                bail!("Unsupported {module} keyword: {key}; use apply for inherited task settings");
            }
        }
    }
    parse_task_parts(task_map, kind, module, args)
}

fn parse_task_parts(
    task_map: Mapping,
    kind: TaskKind,
    module: String,
    mut args: Mapping,
) -> Result<Task> {
    if let Some(extra) = task_map.get(Value::String("args".into())) {
        let extra = extra.as_mapping().context("Task args must be a mapping")?;
        for (key, value) in extra {
            if !key.is_string() {
                bail!("Task args keys must be strings");
            }
            if args.insert(key.clone(), value.clone()).is_some() {
                bail!("Duplicate module argument");
            }
        }
    }
    for keyword in ["changed_when", "failed_when", "until"] {
        if let Some(value) = task_map.get(Value::String(keyword.into())) {
            validate_condition(value, keyword)?;
        }
    }
    for keyword in ["retries", "delay", "timeout", "async", "poll"] {
        if let Some(value) = task_map.get(Value::String(keyword.into())) {
            if !matches!(value, Value::Number(_) | Value::String(_)) {
                bail!("Task {keyword} must be a number or template expression");
            }
        }
    }
    let name = match task_map.get(Value::String("name".to_string())) {
        Some(Value::String(name)) if !name.trim().is_empty() => name.clone(),
        Some(Value::String(_)) => bail!("{} name cannot be empty", kind.label()),
        Some(_) => bail!("{} name must be a string", kind.label()),
        None if matches!(kind, TaskKind::Handler) => bail!("Handler requires a name field"),
        None => module.clone(),
    };

    let become_override = parse_optional_bool(&task_map, "become", kind.label())?;
    let is_become = become_override.unwrap_or(false);
    let become_user_override =
        parse_optional_nonempty_string(&task_map, "become_user", kind.label())?;
    let become_user = become_user_override
        .clone()
        .unwrap_or_else(|| "root".to_string());
    let connection = parse_optional_nonempty_string(&task_map, "connection", kind.label())?;
    let check_mode = parse_optional_bool(&task_map, "check_mode", kind.label())?;

    let register = parse_optional_nonempty_string(&task_map, "register", kind.label())?;
    if let Some(register) = &register {
        crate::vars::validate_name(register, "register")?;
    }

    let when = task_map.get(Value::String("when".to_string())).cloned();
    if let Some(condition) = &when {
        validate_condition(condition, "when")?;
    }

    let notify = parse_string_list(
        task_map.get(Value::String("notify".to_string())),
        "Task notify",
    )?;
    let ignore_errors = parse_optional_bool(&task_map, "ignore_errors", kind.label())?;
    let ignore_unreachable = parse_optional_bool(&task_map, "ignore_unreachable", kind.label())?;
    let no_log = parse_optional_bool(&task_map, "no_log", kind.label())?;
    let vars = parse_variables(task_map.get(Value::String("vars".to_string())), "Task vars")?;

    let loop_value = task_map.get(Value::String("loop".to_string()));
    let with_items = task_map.get(Value::String("with_items".to_string()));
    if loop_value.is_some() && with_items.is_some() {
        bail!("Task cannot specify both 'loop' and 'with_items'");
    }
    let loop_items = loop_value.or(with_items).cloned();
    if let Some(items) = &loop_items {
        if !matches!(items, Value::String(_) | Value::Sequence(_)) {
            bail!("Task loop must be a string expression or sequence");
        }
    }

    let (loop_var_name, index_var_name) = parse_loop_control(
        task_map.get(Value::String("loop_control".to_string())),
        loop_items.is_some(),
    )?;

    debug!("Finished parsing task '{}' with module '{}'", name, module);

    Ok(Task {
        group: None,
        parent: None,
        original: None,
        scope: crate::playbook::TaskScope::default(),
        name,
        module,
        args,
        source_file: None,
        is_become,
        become_override,
        become_user,
        become_user_override,
        connection,
        check_mode,
        register,
        when,
        changed_when: task_map.get(Value::String("changed_when".into())).cloned(),
        failed_when: task_map.get(Value::String("failed_when".into())).cloned(),
        until: task_map.get(Value::String("until".into())).cloned(),
        retries: task_map.get(Value::String("retries".into())).cloned(),
        delay: task_map.get(Value::String("delay".into())).cloned(),
        timeout: task_map.get(Value::String("timeout".into())).cloned(),
        asynchronous: task_map.get(Value::String("async".into())).cloned(),
        poll: task_map.get(Value::String("poll".into())).cloned(),
        environment: parse_environment(&task_map)?,
        notify,
        ignore_errors,
        ignore_unreachable,
        no_log,
        vars,
        tags: Vec::new(),
        loop_items,
        loop_var_name,
        index_var_name,
        delegate_to: parse_optional_nonempty_string(&task_map, "delegate_to", kind.label())?,
        delegate_facts: parse_optional_bool(&task_map, "delegate_facts", kind.label())?,
        throttle: parse_optional_usize(&task_map, "throttle", kind.label())?,
        run_once: parse_optional_bool(&task_map, "run_once", kind.label())?,
        any_errors_fatal: parse_optional_bool(&task_map, "any_errors_fatal", kind.label())?,
    })
}

fn parse_module(task_map: &Mapping, kind: TaskKind) -> Result<(String, Mapping)> {
    let mut candidates = Vec::new();
    for (key, value) in task_map {
        let Value::String(key) = key else {
            bail!("{} keys must be strings", kind.label());
        };
        if SUPPORTED_TASK_KEYWORDS.contains(&key.as_str()) {
            continue;
        }
        if UNSUPPORTED_TASK_KEYWORDS.contains(&key.as_str()) || key.starts_with("with_") {
            bail!(
                "{} keyword '{}' is recognized but not supported",
                kind.label(),
                key
            );
        }
        candidates.push((key, value));
    }

    match candidates.as_slice() {
        [] => bail!("{} doesn't specify a module to execute", kind.label()),
        [candidate] => parse_module_args(candidate.0, candidate.1),
        _ => bail!(
            "{} must specify exactly one module; found: {}",
            kind.label(),
            candidates
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn parse_module_args(module: &str, value: &Value) -> Result<(String, Mapping)> {
    let normalized = if matches!(
        short_name(module),
        "import_tasks" | "include_tasks" | "include_role" | "import_role"
    ) {
        short_name(module)
    } else {
        crate::modules::registry::Builtin::lookup(module)?.name()
    };
    let mut args = Mapping::new();
    match value {
        Value::Null => {}
        Value::String(value)
            if matches!(
                normalized,
                "include_vars" | "import_tasks" | "include_tasks"
            ) =>
        {
            args.insert(Value::String("file".into()), Value::String(value.clone()));
        }
        Value::String(value) if normalized == "meta" => {
            args.insert(Value::String("action".into()), Value::String(value.clone()));
        }
        Value::String(value) if normalized == "debug" => {
            args.insert(
                Value::String("msg".to_string()),
                Value::String(value.clone()),
            );
        }
        Value::String(value) => {
            args.insert(
                Value::String("_raw_params".to_string()),
                Value::String(value.clone()),
            );
        }
        Value::Mapping(value) => {
            for key in value.keys() {
                if !matches!(key, Value::String(_)) {
                    bail!("Module '{}' argument names must be strings", module);
                }
            }
            args = value.clone();
        }
        _ => bail!(
            "Module '{}' arguments must be a string, mapping, or null",
            module
        ),
    }
    Ok((module.to_string(), args))
}

fn parse_environment(map: &Mapping) -> Result<Mapping> {
    let Some(value) = map.get(Value::String("environment".into())) else {
        return Ok(Mapping::new());
    };
    let variables = value
        .as_mapping()
        .context("Environment must be a mapping")?;
    for key in variables.keys() {
        let name = key.as_str().context("Environment names must be strings")?;
        if name.is_empty()
            || name.bytes().enumerate().any(|(index, byte)| {
                !(byte.is_ascii_alphabetic()
                    || byte == b'_'
                    || (index > 0 && byte.is_ascii_digit()))
            })
        {
            bail!("Invalid environment name");
        }
    }
    Ok(variables.clone())
}

fn parse_variables(value: Option<&Value>, label: &str) -> Result<Mapping> {
    let Some(value) = value else {
        return Ok(Mapping::new());
    };
    let Value::Mapping(variables) = value else {
        bail!("{} must be a mapping", label);
    };
    for key in variables.keys() {
        let Value::String(name) = key else {
            bail!("{} names must be strings", label);
        };
        crate::vars::validate_name(name, label)?;
    }
    Ok(variables.clone())
}

fn parse_optional_usize(map: &Mapping, key: &str, label: &str) -> Result<Option<usize>> {
    map.get(Value::String(key.into()))
        .map(|value| {
            value
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .with_context(|| format!("{label} {key} must be a nonnegative integer"))
        })
        .transpose()
}

fn parse_optional_bool(map: &Mapping, key: &str, label: &str) -> Result<Option<bool>> {
    map.get(Value::String(key.to_string()))
        .map(|value| parse_bool(value, &format!("{} {}", label, key)))
        .transpose()
}

fn parse_bool(value: &Value, label: &str) -> Result<bool> {
    match value {
        Value::Bool(value) => Ok(*value),
        Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
            "yes" | "true" | "on" | "1" => Ok(true),
            "no" | "false" | "off" | "0" => Ok(false),
            _ => bail!("{} must be a boolean", label),
        },
        _ => bail!("{} must be a boolean", label),
    }
}

fn parse_optional_nonempty_string(map: &Mapping, key: &str, label: &str) -> Result<Option<String>> {
    match map.get(Value::String(key.to_string())) {
        None => Ok(None),
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(Some(value.clone())),
        Some(Value::String(_)) => bail!("{} {} cannot be empty", label, key),
        Some(_) => bail!("{} {} must be a string", label, key),
    }
}

fn parse_string_list(value: Option<&Value>, label: &str) -> Result<Vec<String>> {
    match value {
        None => Ok(Vec::new()),
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(vec![value.clone()]),
        Some(Value::String(_)) => bail!("{} cannot contain an empty string", label),
        Some(Value::Sequence(values)) => values
            .iter()
            .enumerate()
            .map(|(index, value)| match value {
                Value::String(value) if !value.trim().is_empty() => Ok(value.clone()),
                _ => bail!("{} item {} must be a non-empty string", label, index + 1),
            })
            .collect(),
        Some(_) => bail!("{} must be a string or sequence of strings", label),
    }
}

fn validate_condition(condition: &Value, label: &str) -> Result<()> {
    match condition {
        Value::String(condition) if !condition.trim().is_empty() => Ok(()),
        Value::Bool(_) => Ok(()),
        Value::Sequence(conditions) => {
            for (index, condition) in conditions.iter().enumerate() {
                if !matches!(condition, Value::String(value) if !value.trim().is_empty())
                    && !matches!(condition, Value::Bool(_))
                {
                    bail!(
                        "{} condition {} must be a non-empty string or boolean",
                        label,
                        index + 1
                    );
                }
            }
            Ok(())
        }
        _ => bail!(
            "{} must be a string, boolean, or sequence of conditions",
            label
        ),
    }
}

fn parse_loop_control(
    value: Option<&Value>,
    has_loop: bool,
) -> Result<(Option<String>, Option<String>)> {
    let Some(value) = value else {
        return Ok((None, None));
    };
    if !has_loop {
        bail!("Task loop_control requires loop or with_items");
    }
    let Value::Mapping(control) = value else {
        bail!("Task loop_control must be a mapping");
    };
    for key in control.keys() {
        let Value::String(key) = key else {
            bail!("Task loop_control keys must be strings");
        };
        if !matches!(key.as_str(), "loop_var" | "index_var") {
            bail!("Task loop_control keyword '{}' is not supported", key);
        }
    }
    let loop_var = parse_optional_nonempty_string(control, "loop_var", "Task loop_control")?;
    let index_var = parse_optional_nonempty_string(control, "index_var", "Task loop_control")?;
    if let Some(name) = &loop_var {
        crate::vars::validate_name(name, "loop_var")?;
    }
    if let Some(name) = &index_var {
        crate::vars::validate_name(name, "index_var")?;
    }
    Ok((loop_var, index_var))
}

/// Parse a handler from a YAML mapping (similar to a task).
fn parse_handler(mut handler_map: Mapping, index: usize) -> Result<Handler> {
    let listen = parse_string_list(
        handler_map.get(Value::String("listen".into())),
        "Handler listen",
    )?;
    handler_map.remove(Value::String("listen".into()));
    Ok(Handler {
        task: parse_task(handler_map, index, TaskKind::Handler)?,
        listen,
        hosts: None,
        cached_name: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::Value;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_temp_playbook(content: &str) -> NamedTempFile {
        let mut temp_file = NamedTempFile::new().unwrap();
        write!(temp_file, "{}", content).unwrap();
        temp_file
    }

    #[test]
    fn test_parse_simple_playbook() {
        let content = r#"
---
- name: Test Play
  hosts: localhost
  tasks:
    - name: Test Task
      command: echo hello
"#;
        let temp_file = create_temp_playbook(content);
        let playbook =
            parse_playbook(temp_file.path().to_str().unwrap(), &ParseOptions::default()).unwrap();

        assert_eq!(playbook.plays.len(), 1);
        let play = &playbook.plays[0];
        assert_eq!(play.name, "Test Play");
        assert_eq!(play.hosts, "localhost");
        assert_eq!(play.tasks.len(), 1);

        let task = &play.tasks[0];
        assert_eq!(task.name, "Test Task");
        assert_eq!(task.module, "command");
        assert!(task
            .args
            .contains_key(Value::String("_raw_params".to_string())));
    }

    #[test]
    fn test_parse_task_with_details() {
        let content = r#"
---
- name: Detailed Play
  hosts: all
  tasks:
    - name: Detailed Task
      shell: echo {{ item }}
      become: true
      become_user: admin
      register: shell_result
      when: ansible_os_family == "Debian"
      notify: Restart service
      ignore_errors: yes
      no_log: true
      with_items:
        - one
        - two
"#;
        let temp_file = create_temp_playbook(content);
        let playbook =
            parse_playbook(temp_file.path().to_str().unwrap(), &ParseOptions::default()).unwrap();
        let task = &playbook.plays[0].tasks[0];

        assert_eq!(task.name, "Detailed Task");
        assert_eq!(task.module, "shell");
        assert!(task
            .args
            .contains_key(Value::String("_raw_params".to_string())));
        assert!(task.is_become);
        assert_eq!(task.become_user, "admin");
        assert_eq!(task.register, Some("shell_result".to_string()));
        assert!(task.when.is_some());
        assert_eq!(task.notify, vec!["Restart service".to_string()]);
        assert_eq!(task.ignore_errors, Some(true));
        assert_eq!(task.no_log, Some(true));
        assert!(task.loop_items.is_some());
        if let Some(Value::Sequence(items)) = &task.loop_items {
            assert_eq!(items.len(), 2);
        } else {
            panic!("Expected loop_items to be a sequence");
        }
    }

    #[test]
    fn test_parse_task_args_map() {
        let content = r#"
---
- name: Map Args Play
  hosts: all
  tasks:
    - name: File Task
      file:
        path: /tmp/test
        state: directory
"#;
        let temp_file = create_temp_playbook(content);
        let playbook =
            parse_playbook(temp_file.path().to_str().unwrap(), &ParseOptions::default()).unwrap();
        let task = &playbook.plays[0].tasks[0];

        assert_eq!(task.module, "file");
        assert!(!task
            .args
            .contains_key(Value::String("_raw_params".to_string())));
        assert_eq!(task.args.len(), 2);
        assert!(task.args.contains_key(Value::String("path".to_string())));
        assert!(task.args.contains_key(Value::String("state".to_string())));
    }

    #[test]
    fn test_parse_handler() {
        let content = r#"
---
- name: Handler Play
  hosts: web
  tasks:
    - name: Dummy task
      command: echo
      notify: Restart nginx
  handlers:
    - name: Restart nginx
      service:
        name: nginx
        state: restarted
"#;
        let temp_file = create_temp_playbook(content);
        let playbook =
            parse_playbook(temp_file.path().to_str().unwrap(), &ParseOptions::default()).unwrap();
        let play = &playbook.plays[0];

        assert_eq!(play.handlers.len(), 1);
        let handler_task = &play.handlers[0].task;
        assert_eq!(handler_task.name, "Restart nginx");
        assert_eq!(handler_task.module, "service");
        assert!(handler_task
            .args
            .contains_key(Value::String("name".to_string())));
        assert!(handler_task
            .args
            .contains_key(Value::String("state".to_string())));
    }

    #[test]
    fn test_parse_loop_list() {
        let content = r#"
---
- name: Loop List Play
  hosts: all
  tasks:
    - name: Loop Task
      debug:
        msg: "Item: {{ item }}"
      loop:
        - first
        - second
"#;
        let temp_file = create_temp_playbook(content);
        let playbook =
            parse_playbook(temp_file.path().to_str().unwrap(), &ParseOptions::default()).unwrap();
        let task = &playbook.plays[0].tasks[0];

        assert!(task.loop_items.is_some());
        if let Some(Value::Sequence(items)) = &task.loop_items {
            assert_eq!(items.len(), 2);
            assert_eq!(items[0], Value::String("first".to_string()));
        } else {
            panic!("Expected loop_items to be a sequence");
        }
        assert!(task.loop_var_name.is_none());
        assert!(task.index_var_name.is_none());
    }

    #[test]
    fn test_parse_loop_string_expression() {
        let content = r#"
---
- name: Loop String Play
  hosts: all
  tasks:
    - name: Loop Task
      debug:
        msg: "Item: {{ item }}"
      loop: "{{ my_variable | flatten }}"
"#;
        let temp_file = create_temp_playbook(content);
        let playbook =
            parse_playbook(temp_file.path().to_str().unwrap(), &ParseOptions::default()).unwrap();
        let task = &playbook.plays[0].tasks[0];

        assert!(task.loop_items.is_some());
        assert_eq!(
            task.loop_items,
            Some(Value::String("{{ my_variable | flatten }}".to_string()))
        );
        assert!(task.loop_var_name.is_none());
        assert!(task.index_var_name.is_none());
    }

    #[test]
    fn test_parse_loop_control() {
        let content = r#"
---
- name: Loop Control Play
  hosts: all
  tasks:
    - name: Loop Task
      debug:
        msg: "Index: {{ idx }} Var: {{ element }}"
      loop:
        - a
        - b
      loop_control:
        loop_var: element
        index_var: idx
"#;
        let temp_file = create_temp_playbook(content);
        let playbook =
            parse_playbook(temp_file.path().to_str().unwrap(), &ParseOptions::default()).unwrap();
        let task = &playbook.plays[0].tasks[0];

        assert!(task.loop_items.is_some());
        assert_eq!(task.loop_var_name, Some("element".to_string()));
        assert_eq!(task.index_var_name, Some("idx".to_string()));
    }

    #[test]
    fn test_parse_invalid_yaml() {
        let content = "invalid: yaml: : syntax";
        let temp_file = create_temp_playbook(content);
        let result = parse_playbook(temp_file.path().to_str().unwrap(), &ParseOptions::default());
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Failed to parse YAML"));
    }

    #[test]
    fn test_parse_missing_hosts() {
        let content = r#"
---
- name: No Hosts Play
  tasks:
    - command: echo
"#;
        let temp_file = create_temp_playbook(content);
        let result = parse_playbook(temp_file.path().to_str().unwrap(), &ParseOptions::default());
        assert!(result.is_err());
        // Check the debug representation which includes the full error chain
        let err_debug = format!("{:?}", result.unwrap_err());
        assert!(err_debug.contains("Play requires a hosts field"));
    }

    #[test]
    fn test_missing_play_and_task_names_get_stable_defaults() {
        let content = r#"
---
- hosts: localhost
  tasks:
    - command: echo
"#;
        let temp_file = create_temp_playbook(content);
        let playbook =
            parse_playbook(temp_file.path().to_str().unwrap(), &ParseOptions::default()).unwrap();
        assert_eq!(playbook.plays[0].name, "Play 1");
        assert_eq!(playbook.plays[0].tasks[0].name, "command");
    }

    #[test]
    fn test_parse_missing_task_module() {
        let content = r#"
---
- name: Bad Task Play
  hosts: localhost
  tasks:
    - name: No Module Task
      when: true
      register: foo
"#;
        let temp_file = create_temp_playbook(content);
        let result = parse_playbook(temp_file.path().to_str().unwrap(), &ParseOptions::default());
        assert!(result.is_err());
        let err_debug = format!("{:?}", result.unwrap_err());
        assert!(err_debug.contains("Task doesn't specify a module"));
    }

    #[test]
    fn rejects_empty_and_scalar_documents() {
        for content in ["", "---\nnull\n", "---\n42\n", "---\n[]\n"] {
            let temp_file = create_temp_playbook(content);
            assert!(
                parse_playbook(temp_file.path().to_str().unwrap(), &ParseOptions::default())
                    .is_err(),
                "content should have been rejected: {content:?}"
            );
        }
    }

    #[test]
    fn rejects_non_mapping_play_task_and_handler_entries() {
        for content in [
            "---\n- not-a-play\n",
            "---\n- hosts: all\n  tasks: not-a-list\n",
            "---\n- hosts: all\n  tasks:\n    - not-a-task\n",
            "---\n- hosts: all\n  handlers:\n    - not-a-handler\n",
        ] {
            let temp_file = create_temp_playbook(content);
            assert!(
                parse_playbook(temp_file.path().to_str().unwrap(), &ParseOptions::default())
                    .is_err(),
                "content should have been rejected: {content:?}"
            );
        }
    }

    #[test]
    fn handler_name_remains_required_without_listen_support() {
        let content = r#"
---
- hosts: all
  handlers:
    - service:
        name: nginx
        state: restarted
"#;
        let temp_file = create_temp_playbook(content);
        let error = parse_playbook(temp_file.path().to_str().unwrap(), &ParseOptions::default())
            .unwrap_err();
        assert!(format!("{error:?}").contains("Handler requires a name field"));
    }

    #[test]
    fn handler_keywords_use_the_same_task_result_semantics() {
        for keyword in ["register: result", "notify: another", "ignore_errors: true"] {
            let content = format!("- hosts: all\n  handlers:\n    - name: handler\n      command: 'true'\n      {keyword}\n");
            let file = create_temp_playbook(&content);
            let parsed =
                parse_playbook(file.path().to_str().unwrap(), &ParseOptions::default()).unwrap();
            let handler = &parsed.plays[0].handlers[0].task;
            match keyword {
                "register: result" => assert_eq!(handler.register.as_deref(), Some("result")),
                "notify: another" => assert_eq!(handler.notify, ["another"]),
                _ => assert_eq!(handler.ignore_errors, Some(true)),
            }
        }
    }

    #[test]
    fn tags_are_rejected_until_tag_selection_is_implemented() {
        for content in [
            "---\n- hosts: all\n  tags: deploy\n  tasks: []\n",
            "---\n- hosts: all\n  tasks:\n    - command: true\n      tags: deploy\n",
        ] {
            let temp_file = create_temp_playbook(content);
            let error =
                parse_playbook(temp_file.path().to_str().unwrap(), &ParseOptions::default())
                    .unwrap_err();
            assert!(format!("{error:?}").contains("recognized but not supported"));
        }
    }

    #[test]
    fn task_must_contain_exactly_one_module() {
        let content = r#"
---
- hosts: all
  tasks:
    - command: echo one
      debug: two
"#;
        let temp_file = create_temp_playbook(content);
        let error = parse_playbook(temp_file.path().to_str().unwrap(), &ParseOptions::default())
            .unwrap_err();
        assert!(format!("{error:?}").contains("must specify exactly one module"));
    }

    #[test]
    fn recognized_but_unsupported_task_keywords_are_not_modules() {
        for keyword in ["collections", "diff"] {
            let content = format!(
                "---\n- hosts: all\n  tasks:\n    - command: echo\n      {keyword}: true\n"
            );
            let temp_file = create_temp_playbook(&content);
            let error =
                parse_playbook(temp_file.path().to_str().unwrap(), &ParseOptions::default())
                    .unwrap_err();
            let message = format!("{error:?}");
            assert!(
                message.contains("recognized but not supported"),
                "{message}"
            );
            assert!(message.contains(keyword), "{message}");
        }
    }

    #[test]
    fn parses_task_vars_when_list_and_tristate_options() {
        let content = r#"
---
- hosts: all
  become: true
  tasks:
    - debug: "{{ task_message }}"
      become: false
      check_mode: true
      vars:
        task_message: hello
      when:
        - true
        - inventory_hostname == "node"
"#;
        let temp_file = create_temp_playbook(content);
        let playbook =
            parse_playbook(temp_file.path().to_str().unwrap(), &ParseOptions::default()).unwrap();
        let task = &playbook.plays[0].tasks[0];
        assert_eq!(task.become_override, Some(false));
        assert!(!task.is_become);
        assert_eq!(task.check_mode, Some(true));
        assert_eq!(
            task.vars.get(Value::String("task_message".to_string())),
            Some(&Value::String("hello".to_string()))
        );
        assert!(matches!(task.when, Some(Value::Sequence(ref values)) if values.len() == 2));
    }

    #[test]
    fn rejects_invalid_common_keyword_types_and_loop_combinations() {
        for content in [
            "---\n- hosts: all\n  tasks:\n    - command: echo\n      vars: nope\n",
            "---\n- hosts: all\n  tasks:\n    - command: echo\n      notify: [valid, 2]\n",
            "---\n- hosts: all\n  tasks:\n    - command: echo\n      when: {bad: condition}\n",
            "---\n- hosts: all\n  tasks:\n    - command: echo\n      loop: [one]\n      with_items: [two]\n",
            "---\n- hosts: all\n  tasks:\n    - command: echo\n      loop_control: {loop_var: item}\n",
        ] {
            let temp_file = create_temp_playbook(content);
            assert!(
                parse_playbook(temp_file.path().to_str().unwrap(), &ParseOptions::default()).is_err(),
                "content should have been rejected: {content:?}"
            );
        }
    }

    #[test]
    fn parses_multiple_yaml_documents_without_skipping_them() {
        let content = r#"
---
- hosts: first
  tasks:
    - debug: first
---
hosts: second
tasks:
  - debug: second
"#;
        let temp_file = create_temp_playbook(content);
        let playbook =
            parse_playbook(temp_file.path().to_str().unwrap(), &ParseOptions::default()).unwrap();
        assert_eq!(playbook.plays.len(), 2);
        assert_eq!(playbook.plays[0].name, "Play 1");
        assert_eq!(playbook.plays[1].name, "Play 2");
    }

    #[test]
    fn fail_fast_is_scoped_to_each_play() {
        let content = r#"
---
- hosts: first
  fail_fast: true
  tasks: []
- hosts: second
  fail_fast: false
  tasks: []
"#;
        let temp_file = create_temp_playbook(content);
        let playbook =
            parse_playbook(temp_file.path().to_str().unwrap(), &ParseOptions::default()).unwrap();
        assert!(playbook.plays[0].fail_fast);
        assert!(!playbook.plays[1].fail_fast);
    }

    #[test]
    fn parses_agent_update_compatibility_surface_and_gather_facts_default() {
        let content = r#"
---
- name: Update agent
  hosts: all
  gather_facts: true
  vars:
    architecture: "{{ 'aarch64' if ansible_architecture in ['aarch64', 'arm64'] else 'x86_64' }}"
  tasks:
    - ansible.builtin.copy:
        dest: /tmp/config
        content: config
        force: false
    - ansible.builtin.stat:
        path: /tmp/agent
        checksum_algorithm: sha1
      register: installed
    - ansible.builtin.get_url:
        url: http://example.invalid/agent
        dest: /tmp/agent.download
        force: true
        checksum: "{{ checksum | default(omit) }}"
      register: downloaded
    - ansible.builtin.systemd:
        name: agent.service
        state: started
        enabled: true
        daemon_reload: true
"#;
        let temp_file = create_temp_playbook(content);
        let playbook =
            parse_playbook(temp_file.path().to_str().unwrap(), &ParseOptions::default()).unwrap();

        assert!(playbook.plays[0].gather_facts);
        assert_eq!(playbook.plays[0].tasks.len(), 4);

        let defaulted = create_temp_playbook("---\n- hosts: all\n  tasks: []\n");
        assert!(
            parse_playbook(defaulted.path().to_str().unwrap(), &ParseOptions::default())
                .unwrap()
                .plays[0]
                .gather_facts
        );
        let disabled =
            create_temp_playbook("---\n- hosts: all\n  gather_facts: false\n  tasks: []\n");
        assert!(
            !parse_playbook(disabled.path().to_str().unwrap(), &ParseOptions::default())
                .unwrap()
                .plays[0]
                .gather_facts
        );
    }
}
