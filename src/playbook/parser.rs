use anyhow::{bail, Context, Result};
use log::debug;
use serde::Deserialize;
use serde_yaml::{Mapping, Value};
use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::playbook::{Handler, Play, Task};

/// The main Playbook structure
#[derive(Debug)]
pub struct Playbook {
    pub plays: Vec<Play>,
}

/// Parse an Ansible playbook YAML file
pub fn parse_playbook(playbook_path: &str) -> Result<Playbook> {
    debug!("Parsing playbook file: {}", playbook_path);

    let path = Path::new(playbook_path);
    let mut file =
        File::open(path).context(format!("Failed to open playbook file: {}", playbook_path))?;

    let mut content = String::new();
    file.read_to_string(&mut content)
        .context("Failed to read playbook file content")?;

    let mut plays = Vec::new();
    let mut document_count = 0;

    for (doc_index, document) in serde_yaml::Deserializer::from_str(&content).enumerate() {
        document_count += 1;
        let doc = Value::deserialize(document)
            .with_context(|| format!("Failed to parse YAML document {}", doc_index + 1))?;
        match doc {
            Value::Sequence(items) => {
                if items.is_empty() {
                    bail!("YAML document {} contains no plays", doc_index + 1);
                }
                debug!(
                    "Processing YAML document {} with {} play entries",
                    doc_index + 1,
                    items.len()
                );

                for (play_index, play_value) in items.into_iter().enumerate() {
                    let Value::Mapping(play_map) = play_value else {
                        bail!(
                            "Play {} in YAML document {} must be a mapping",
                            play_index + 1,
                            doc_index + 1
                        );
                    };
                    debug!(
                        "Processing play {} in document {}",
                        play_index + 1,
                        doc_index + 1
                    );
                    let play = parse_play(play_map, plays.len() + 1).with_context(|| {
                        format!(
                            "Failed to parse play {} in document {}",
                            play_index + 1,
                            doc_index + 1
                        )
                    })?;
                    plays.push(play);
                }
            }
            Value::Mapping(doc_map) => {
                debug!("Processing document {} as a single play", doc_index + 1);
                let play = parse_play(doc_map, plays.len() + 1).with_context(|| {
                    format!("Failed to parse play in document {}", doc_index + 1)
                })?;
                plays.push(play);
            }
            Value::Null => bail!("YAML document {} is empty", doc_index + 1),
            _ => bail!(
                "YAML document {} must contain a play mapping or a sequence of play mappings",
                doc_index + 1
            ),
        }
    }

    if document_count == 0 || plays.is_empty() {
        bail!("Playbook contains no plays");
    }

    debug!("Finished parsing playbook with {} plays", plays.len());

    Ok(Playbook { plays })
}

const SUPPORTED_PLAY_KEYWORDS: &[&str] = &[
    "name",
    "hosts",
    "tasks",
    "handlers",
    "vars",
    "become",
    "become_user",
    "connection",
    "fail_fast",
];

const UNSUPPORTED_PLAY_KEYWORDS: &[&str] = &[
    "any_errors_fatal",
    "become_exe",
    "become_flags",
    "become_method",
    "check_mode",
    "collections",
    "debugger",
    "diff",
    "environment",
    "fact_path",
    "force_handlers",
    "gather_facts",
    "gather_subset",
    "gather_timeout",
    "ignore_errors",
    "no_log",
    "max_fail_percentage",
    "module_defaults",
    "order",
    "port",
    "post_tasks",
    "pre_tasks",
    "remote_user",
    "roles",
    "run_once",
    "serial",
    "strategy",
    "tags",
    "throttle",
    "timeout",
    "vars_files",
    "vars_prompt",
];

const SUPPORTED_TASK_KEYWORDS: &[&str] = &[
    "name",
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
    "any_errors_fatal",
    "args",
    "async",
    "become_exe",
    "become_flags",
    "become_method",
    "block",
    "changed_when",
    "collections",
    "debugger",
    "delay",
    "delegate_facts",
    "delegate_to",
    "diff",
    "environment",
    "failed_when",
    "ignore_unreachable",
    "listen",
    "module_defaults",
    "poll",
    "port",
    "remote_user",
    "rescue",
    "retries",
    "run_once",
    "tags",
    "throttle",
    "timeout",
    "until",
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

/// Parse an individual play from a YAML mapping.
fn parse_play(play_map: Mapping, ordinal: usize) -> Result<Play> {
    debug!("Parsing play definition");
    validate_play_keys(&play_map)?;

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

    let tasks: Vec<Task> = parse_task_list(
        play_map.get(Value::String("tasks".to_string())),
        TaskKind::Task,
    )?
    .into_iter()
    .map(|task| task.task)
    .collect();
    let handlers = parse_task_list(
        play_map.get(Value::String("handlers".to_string())),
        TaskKind::Handler,
    )?;

    let vars = parse_variables(play_map.get(Value::String("vars".to_string())), "Play vars")?;
    let become_override = parse_optional_bool(&play_map, "become", "Play")?;
    let is_become = become_override.unwrap_or(false);
    let become_user_override = parse_optional_nonempty_string(&play_map, "become_user", "Play")?;
    let become_user = become_user_override
        .clone()
        .unwrap_or_else(|| "root".to_string());
    let connection = parse_optional_nonempty_string(&play_map, "connection", "Play")?;
    let fail_fast = parse_optional_bool(&play_map, "fail_fast", "Play")?.unwrap_or(false);

    debug!(
        "Finished parsing play '{}' with {} tasks and {} handlers",
        name,
        tasks.len(),
        handlers.len()
    );

    Ok(Play {
        name,
        hosts,
        tasks,
        handlers,
        vars,
        become_override,
        is_become,
        become_user,
        become_user_override,
        fail_fast,
        connection,
        tags: Vec::new(),
    })
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

fn parse_task_list(value: Option<&Value>, kind: TaskKind) -> Result<Vec<Handler>> {
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
        let handler = match kind {
            TaskKind::Task => Handler {
                task: parse_task(mapping.clone(), index, kind)?,
            },
            TaskKind::Handler => parse_handler(mapping.clone(), index)?,
        };
        parsed.push(handler);
    }
    Ok(parsed)
}

/// Parse a task or handler from a YAML mapping.
fn parse_task(task_map: Mapping, index: usize, kind: TaskKind) -> Result<Task> {
    debug!("Parsing {} definition at index {}", kind.label(), index);

    let (module, args) = parse_module(&task_map, kind)?;
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
    if matches!(kind, TaskKind::Handler) && register.is_some() {
        bail!("Handler keyword 'register' is recognized but not supported");
    }
    if let Some(register) = &register {
        validate_variable_name(register, "register")?;
    }

    let when = task_map.get(Value::String("when".to_string())).cloned();
    if let Some(condition) = &when {
        validate_condition(condition, "when")?;
    }

    let notify = parse_string_list(
        task_map.get(Value::String("notify".to_string())),
        "Task notify",
    )?;
    if matches!(kind, TaskKind::Handler) && !notify.is_empty() {
        bail!("Handler keyword 'notify' is recognized but not supported");
    }
    let ignore_errors =
        parse_optional_bool(&task_map, "ignore_errors", kind.label())?.unwrap_or(false);
    if matches!(kind, TaskKind::Handler) && ignore_errors {
        bail!("Handler keyword 'ignore_errors' is recognized but not supported");
    }
    let no_log = parse_optional_bool(&task_map, "no_log", kind.label())?.unwrap_or(false);
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
        name,
        module,
        args,
        is_become,
        become_override,
        become_user,
        become_user_override,
        connection,
        check_mode,
        register,
        when,
        notify,
        ignore_errors,
        no_log,
        vars,
        tags: Vec::new(),
        loop_items,
        loop_var_name,
        index_var_name,
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
    let normalized = module
        .strip_prefix("ansible.builtin.")
        .or_else(|| module.strip_prefix("ansible.legacy."))
        .unwrap_or(module);
    if !matches!(
        normalized,
        "command"
            | "shell"
            | "debug"
            | "copy"
            | "file"
            | "template"
            | "package"
            | "service"
            | "lineinfile"
            | "user"
    ) {
        bail!("Unsupported module: {}", module);
    }
    let mut args = Mapping::new();
    match value {
        Value::Null => {}
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
        validate_variable_name(name, label)?;
    }
    Ok(variables.clone())
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

fn validate_variable_name(name: &str, label: &str) -> Result<()> {
    let mut characters = name.chars();
    let valid_start = characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic());
    if !valid_start
        || !characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        bail!("{} contains invalid variable name '{}'", label, name);
    }
    Ok(())
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
        validate_variable_name(name, "loop_var")?;
    }
    if let Some(name) = &index_var {
        validate_variable_name(name, "index_var")?;
    }
    Ok((loop_var, index_var))
}

/// Parse a handler from a YAML mapping (similar to a task).
fn parse_handler(handler_map: Mapping, index: usize) -> Result<Handler> {
    Ok(Handler {
        task: parse_task(handler_map, index, TaskKind::Handler)?,
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
        let playbook = parse_playbook(temp_file.path().to_str().unwrap()).unwrap();

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
        let playbook = parse_playbook(temp_file.path().to_str().unwrap()).unwrap();
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
        assert!(task.ignore_errors);
        assert!(task.no_log);
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
        let playbook = parse_playbook(temp_file.path().to_str().unwrap()).unwrap();
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
        let playbook = parse_playbook(temp_file.path().to_str().unwrap()).unwrap();
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
        let playbook = parse_playbook(temp_file.path().to_str().unwrap()).unwrap();
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
        let playbook = parse_playbook(temp_file.path().to_str().unwrap()).unwrap();
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
        let playbook = parse_playbook(temp_file.path().to_str().unwrap()).unwrap();
        let task = &playbook.plays[0].tasks[0];

        assert!(task.loop_items.is_some());
        assert_eq!(task.loop_var_name, Some("element".to_string()));
        assert_eq!(task.index_var_name, Some("idx".to_string()));
    }

    #[test]
    fn test_parse_invalid_yaml() {
        let content = "invalid: yaml: : syntax";
        let temp_file = create_temp_playbook(content);
        let result = parse_playbook(temp_file.path().to_str().unwrap());
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
        let result = parse_playbook(temp_file.path().to_str().unwrap());
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
        let playbook = parse_playbook(temp_file.path().to_str().unwrap()).unwrap();
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
        let result = parse_playbook(temp_file.path().to_str().unwrap());
        assert!(result.is_err());
        let err_debug = format!("{:?}", result.unwrap_err());
        assert!(err_debug.contains("Task doesn't specify a module"));
    }

    #[test]
    fn rejects_empty_and_scalar_documents() {
        for content in ["", "---\nnull\n", "---\n42\n", "---\n[]\n"] {
            let temp_file = create_temp_playbook(content);
            assert!(
                parse_playbook(temp_file.path().to_str().unwrap()).is_err(),
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
                parse_playbook(temp_file.path().to_str().unwrap()).is_err(),
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
        let error = parse_playbook(temp_file.path().to_str().unwrap()).unwrap_err();
        assert!(format!("{error:?}").contains("Handler requires a name field"));
    }

    #[test]
    fn handler_keywords_without_execution_semantics_are_rejected() {
        for keyword in ["register: result", "notify: another", "ignore_errors: true"] {
            let content = format!(
                "---\n- hosts: all\n  handlers:\n    - name: handler\n      command: 'true'\n      {keyword}\n"
            );
            let temp_file = create_temp_playbook(&content);
            let error = parse_playbook(temp_file.path().to_str().unwrap()).unwrap_err();
            assert!(
                format!("{error:#}").contains("recognized but not supported"),
                "{error:#}"
            );
        }
    }

    #[test]
    fn tags_are_rejected_until_tag_selection_is_implemented() {
        for content in [
            "---\n- hosts: all\n  tags: deploy\n  tasks: []\n",
            "---\n- hosts: all\n  tasks:\n    - command: true\n      tags: deploy\n",
        ] {
            let temp_file = create_temp_playbook(content);
            let error = parse_playbook(temp_file.path().to_str().unwrap()).unwrap_err();
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
        let error = parse_playbook(temp_file.path().to_str().unwrap()).unwrap_err();
        assert!(format!("{error:?}").contains("must specify exactly one module"));
    }

    #[test]
    fn recognized_but_unsupported_task_keywords_are_not_modules() {
        for keyword in [
            "changed_when",
            "failed_when",
            "delegate_to",
            "environment",
            "retries",
            "delay",
            "until",
            "run_once",
        ] {
            let content = format!(
                "---\n- hosts: all\n  tasks:\n    - command: echo\n      {keyword}: true\n"
            );
            let temp_file = create_temp_playbook(&content);
            let error = parse_playbook(temp_file.path().to_str().unwrap()).unwrap_err();
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
        let playbook = parse_playbook(temp_file.path().to_str().unwrap()).unwrap();
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
                parse_playbook(temp_file.path().to_str().unwrap()).is_err(),
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
        let playbook = parse_playbook(temp_file.path().to_str().unwrap()).unwrap();
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
        let playbook = parse_playbook(temp_file.path().to_str().unwrap()).unwrap();
        assert!(playbook.plays[0].fail_fast);
        assert!(!playbook.plays[1].fail_fast);
    }
}
