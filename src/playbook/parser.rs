use std::fs::File;
use std::io::Read;
use std::path::Path;
use anyhow::{Result, Context};
use serde_yaml::{Value, Mapping};
use log::{debug, warn};

use crate::playbook::{Play, Task, Handler};

/// The main Playbook structure
pub struct Playbook {
    pub plays: Vec<Play>,
    pub fail_fast: bool,
}

/// Parse an Ansible playbook YAML file
pub fn parse_playbook(playbook_path: &str) -> Result<Playbook> {
    debug!("Parsing playbook file: {}", playbook_path);
    
    let path = Path::new(playbook_path);
    let mut file = File::open(path)
        .context(format!("Failed to open playbook file: {}", playbook_path))?;
    
    let mut content = String::new();
    file.read_to_string(&mut content)
        .context("Failed to read playbook file content")?;
    
    let yaml_docs: Vec<Value> = serde_yaml::from_str(&content)
        .context("Failed to parse YAML content")?;
    
    debug!("Parsed {} YAML documents from playbook", yaml_docs.len());
    
    let mut plays = Vec::new();
    let mut fail_fast = false;
    
    for (doc_index, doc) in yaml_docs.into_iter().enumerate() {
        match doc {
            Value::Sequence(items) => {
                debug!("Processing YAML document {} with {} play entries", doc_index, items.len());
                
                for (play_index, play_value) in items.into_iter().enumerate() {
                    match play_value {
                        Value::Mapping(play_map) => {
                            debug!("Processing play {} in document {}", play_index, doc_index);
                            let play = parse_play(play_map)
                                .context(format!("Failed to parse play {} in document {}", play_index, doc_index))?;
                            plays.push(play);
                        },
                        _ => {
                            warn!("Skipping non-mapping play entry at index {} in document {}", play_index, doc_index);
                        }
                    }
                }
            },
            Value::Mapping(doc_map) => {
                debug!("Processing document {} as a single play", doc_index);
                let doc_map_clone = doc_map.clone();
                let play = parse_play(doc_map)
                    .context(format!("Failed to parse play in document {}", doc_index))?;
                plays.push(play);
                
                if let Some(Value::Bool(fast_fail)) = doc_map_clone.get(&Value::String("fail_fast".to_string())) {
                    fail_fast = *fast_fail;
                }
            },
            _ => {
                warn!("Skipping unsupported YAML document at index {}", doc_index);
            }
        }
    }
    
    debug!("Finished parsing playbook with {} plays", plays.len());
    
    Ok(Playbook {
        plays,
        fail_fast,
    })
}

/// Parse an individual play from a YAML mapping
fn parse_play(play_map: Mapping) -> Result<Play> {
    debug!("Parsing play definition");
    
    // Play name is required
    let name = match play_map.get(&Value::String("name".to_string())) {
        Some(Value::String(name)) => name.clone(),
        Some(_) => return Err(anyhow::anyhow!("Play name must be a string")),
        None => return Err(anyhow::anyhow!("Play requires a name field")),
    };
    
    // Hosts are required
    let hosts = match play_map.get(&Value::String("hosts".to_string())) {
        Some(Value::String(hosts)) => hosts.clone(),
        Some(_) => return Err(anyhow::anyhow!("Hosts must be a string")),
        None => return Err(anyhow::anyhow!("Play requires a hosts field")),
    };
    
    // Process tasks
    let mut tasks = Vec::new();
    if let Some(Value::Sequence(task_seq)) = play_map.get(&Value::String("tasks".to_string())) {
        for (task_index, task_value) in task_seq.iter().enumerate() {
            match task_value {
                Value::Mapping(task_map) => {
                    let task = parse_task(task_map.clone(), task_index)
                        .context(format!("Failed to parse task at index {}", task_index))?;
                    tasks.push(task);
                },
                _ => {
                    warn!("Skipping non-mapping task at index {}", task_index);
                }
            }
        }
    }
    
    // Process handlers
    let mut handlers = Vec::new();
    if let Some(Value::Sequence(handler_seq)) = play_map.get(&Value::String("handlers".to_string())) {
        for (handler_index, handler_value) in handler_seq.iter().enumerate() {
            match handler_value {
                Value::Mapping(handler_map) => {
                    let handler = parse_handler(handler_map.clone(), handler_index)
                        .context(format!("Failed to parse handler at index {}", handler_index))?;
                    handlers.push(handler);
                },
                _ => {
                    warn!("Skipping non-mapping handler at index {}", handler_index);
                }
            }
        }
    }
    
    // Variables are optional
    let mut vars = Mapping::new();
    if let Some(Value::Mapping(var_map)) = play_map.get(&Value::String("vars".to_string())) {
        vars = var_map.clone();
    }
    
    // Check for become (privilege escalation)
    let mut is_become = false;
    let mut become_user = "root".to_string();
    
    if let Some(Value::Bool(b)) = play_map.get(&Value::String("become".to_string())) {
        is_become = *b;
    }
    
    if let Some(Value::String(user)) = play_map.get(&Value::String("become_user".to_string())) {
        become_user = user.clone();
    }
    
    // Check for tags
    let mut tags = Vec::new();
    if let Some(Value::Sequence(tag_seq)) = play_map.get(&Value::String("tags".to_string())) {
        for tag_value in tag_seq {
            if let Value::String(tag) = tag_value {
                tags.push(tag.clone());
            }
        }
    } else if let Some(Value::String(tag)) = play_map.get(&Value::String("tags".to_string())) {
        tags.push(tag.clone());
    }
    
    debug!("Finished parsing play '{}' with {} tasks and {} handlers", 
           name, tasks.len(), handlers.len());
    
    Ok(Play {
        name,
        hosts,
        tasks,
        handlers,
        vars,
        is_become,
        become_user,
        tags,
    })
}

/// Parse a task from a YAML mapping
fn parse_task(task_map: Mapping, index: usize) -> Result<Task> {
    debug!("Parsing task definition at index {}", index);
    
    // Name is required for clarity
    let name = match task_map.get(&Value::String("name".to_string())) {
        Some(Value::String(name)) => name.clone(),
        Some(_) => return Err(anyhow::anyhow!("Task name must be a string")),
        None => return Err(anyhow::anyhow!("Task requires a name field")),
    };
    
    // Find the module and arguments
    let mut module = String::new();
    let mut args = Mapping::new();
    
    // In Ansible, the module can be specified in various ways:
    // 1. As a direct key in the task, like "command: uptime"
    // 2. As a dictionary, like "shell: { cmd: 'echo foo', chdir: '/tmp' }"
    
    // Check each key to see if it's a module name
    for (key, value) in &task_map {
        if let Value::String(key_str) = key {
            // Skip common task fields that aren't modules
            if ["name", "become", "become_user", "register", "when", "tags", 
                "notify", "ignore_errors", "vars", "with_items", "loop", "loop_control"].contains(&key_str.as_str()) {
                continue;
            }
            
            // Found a potential module
            module = key_str.clone();

            // Parse the module args based on the type of the value associated with the module key
            match value {
                Value::String(string_val) => {
                    // Value is a simple string
                    if module == "debug" {
                        // For debug, string shorthand means 'msg'
                        args.insert(Value::String("msg".to_string()), Value::String(string_val.clone()));
                    } else {
                        // For command/shell etc., string shorthand means '_raw_params'
                        args.insert(Value::String("_raw_params".to_string()), Value::String(string_val.clone()));
                    }
                },
                Value::Mapping(map_val) => {
                    // Value is a map. Use this map directly as the arguments.
                    // This handles both complex args (e.g., file: { path: ... }) 
                    // and shorthand maps (e.g., debug: { var: ... })
                    args = map_val.clone();
                },
                _ => {
                     // Unsupported argument type for a module key
                     return Err(anyhow::anyhow!("Unsupported value type for module '{}': {:?}", module, value));
                }
            }
            
            // Only process the first module found
            break;
        }
    }
    
    if module.is_empty() {
        return Err(anyhow::anyhow!("Task doesn't specify a module to execute"));
    }
    
    // Check for become (privilege escalation)
    let mut is_become = false;
    let mut become_user = "root".to_string();
    
    if let Some(Value::Bool(b)) = task_map.get(&Value::String("become".to_string())) {
        is_become = *b;
    }
    
    if let Some(Value::String(user)) = task_map.get(&Value::String("become_user".to_string())) {
        become_user = user.clone();
    }
    
    // Check for register variable
    let mut register = None;
    if let Some(Value::String(reg)) = task_map.get(&Value::String("register".to_string())) {
        register = Some(reg.clone());
    }
    
    // Check for conditional
    let mut when = None;
    if let Some(when_value) = task_map.get(&Value::String("when".to_string())) {
        when = Some(when_value.clone());
    }
    
    // Check for notification handlers
    let mut notify = Vec::new();
    if let Some(Value::Sequence(notify_seq)) = task_map.get(&Value::String("notify".to_string())) {
        for notify_value in notify_seq {
            if let Value::String(handler) = notify_value {
                notify.push(handler.clone());
            }
        }
    } else if let Some(Value::String(handler)) = task_map.get(&Value::String("notify".to_string())) {
        notify.push(handler.clone());
    }
    
    // Check for ignore_errors
    let mut ignore_errors = false;
    if let Some(value) = task_map.get(&Value::String("ignore_errors".to_string())) {
        ignore_errors = match value {
            Value::Bool(b) => *b,
            Value::String(s) => match s.to_lowercase().as_str() {
                "yes" | "true" => true,
                _ => false, // Default to false for unrecognized strings or "no"
            },
            _ => false, // Default to false for other types
        };
    }
    
    // Check for tags
    let mut tags = Vec::new();
    if let Some(Value::Sequence(tag_seq)) = task_map.get(&Value::String("tags".to_string())) {
        for tag_value in tag_seq {
            if let Value::String(tag) = tag_value {
                tags.push(tag.clone());
            }
        }
    } else if let Some(Value::String(tag)) = task_map.get(&Value::String("tags".to_string())) {
        tags.push(tag.clone());
    }
    
    // Handle loops
    let mut loop_items = None;
    if let Some(items) = task_map.get(&Value::String("loop".to_string())) {
        loop_items = Some(items.clone());
    } else if let Some(items) = task_map.get(&Value::String("with_items".to_string())) {
        loop_items = Some(items.clone());
    }

    // Handle loop_control
    let mut loop_var_name = None;
    let mut index_var_name = None;
    if let Some(Value::Mapping(lc_map)) = task_map.get(&Value::String("loop_control".to_string())) {
        if let Some(Value::String(lv)) = lc_map.get(&Value::String("loop_var".to_string())) {
            loop_var_name = Some(lv.clone());
        }
        if let Some(Value::String(iv)) = lc_map.get(&Value::String("index_var".to_string())) {
            index_var_name = Some(iv.clone());
        }
    }
    
    debug!("Finished parsing task '{}' with module '{}'", name, module);
    
    Ok(Task {
        name,
        module,
        args,
        is_become,
        become_user,
        register,
        when,
        notify,
        ignore_errors,
        tags,
        loop_items,
        loop_var_name,
        index_var_name,
    })
}

/// Parse a handler from a YAML mapping (similar to a task)
fn parse_handler(handler_map: Mapping, index: usize) -> Result<Handler> {
    debug!("Parsing handler definition at index {}", index);
    
    // A handler is essentially a task that is triggered by notifications
    let task = parse_task(handler_map, index)?;
    
    Ok(Handler {
        task,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use std::io::Write;
    use serde_yaml::Value;

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
        assert!(task.args.contains_key(&Value::String("_raw_params".to_string())));
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
      with_items:
        - one
        - two
"#;
        let temp_file = create_temp_playbook(content);
        let playbook = parse_playbook(temp_file.path().to_str().unwrap()).unwrap();
        let task = &playbook.plays[0].tasks[0];

        assert_eq!(task.name, "Detailed Task");
        assert_eq!(task.module, "shell");
        assert!(task.args.contains_key(&Value::String("_raw_params".to_string())));
        assert_eq!(task.is_become, true);
        assert_eq!(task.become_user, "admin");
        assert_eq!(task.register, Some("shell_result".to_string()));
        assert!(task.when.is_some());
        assert_eq!(task.notify, vec!["Restart service".to_string()]);
        assert_eq!(task.ignore_errors, true);
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
        assert!(!task.args.contains_key(&Value::String("_raw_params".to_string())));
        assert_eq!(task.args.len(), 2);
        assert!(task.args.contains_key(&Value::String("path".to_string())));
        assert!(task.args.contains_key(&Value::String("state".to_string())));
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
        assert!(handler_task.args.contains_key(&Value::String("name".to_string())));
        assert!(handler_task.args.contains_key(&Value::String("state".to_string())));
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
        assert_eq!(task.loop_items, Some(Value::String("{{ my_variable | flatten }}".to_string())));
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
} 