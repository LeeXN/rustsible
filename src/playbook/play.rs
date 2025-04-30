use anyhow::Result;
use log::{info, debug, error};
use serde_yaml::{Value, Mapping};
use colored::Colorize;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::inventory::Host;
use crate::playbook::{Task, TaskResult, Handler};

/// Play structure representing a set of tasks to run on hosts
#[derive(Debug, Clone)]
pub struct Play {
    pub name: String,
    pub hosts: String,
    pub tasks: Vec<Task>,
    pub handlers: Vec<Handler>,
    pub vars: Mapping,
    pub is_become: bool,  // renamed from 'become' to avoid Rust keyword
    pub become_user: String,
    #[allow(dead_code)]
    pub tags: Vec<String>,  // Keep this for future use
}

impl Play {
    pub fn execute(&self, hosts: &[Host]) -> Result<()> {
        let start_time = Instant::now();
        info!("PLAY [{}] on {} hosts", self.name, hosts.len());
        println!("\n{}", format!("PLAY [{}]", self.name).bold());
        
        // Track task results for the play recap
        let mut _ok_hosts = 0;
        let mut failed_hosts = HashSet::new();
        let mut changed_hosts = HashSet::new();
        let mut _skipped_hosts = 0;

        // Track which handlers have been notified
        let mut notified_handlers: HashSet<String> = HashSet::new();
        
        // Execute all tasks in order
        for (task_index, task) in self.tasks.iter().enumerate() {
            debug!("Executing task {} of {}: {}", 
                  task_index + 1, self.tasks.len(), task.name);
            
            // 更接近ansible风格的任务标题
            println!("\nTASK [{}] {}", task.name, "*".repeat(80 - task.name.len() - 8).dimmed());
            
            let mut task_results = Vec::new();
            
            // Create a task with play's become settings if task doesn't override
            let mut effective_task = task.clone();
            if !effective_task.is_become && self.is_become {
                effective_task.is_become = self.is_become;
                effective_task.become_user = self.become_user.clone();
            }
            
            for host in hosts {
                // Prepare host-specific variables by merging play vars with host vars
                let mut host_vars: HashMap<String, Value> = HashMap::new();
                
                // First add play variables
                for (key, value) in &self.vars {
                    if let Value::String(k) = key {
                        host_vars.insert(k.clone(), value.clone());
                    }
                }
                
                // Then add host variables (they take precedence over play vars)
                // Add host's own variables
                for (key, value) in &host.variables {
                    host_vars.insert(key.clone(), Value::String(value.clone()));
                }
                
                // Add host's inherited variables (from groups)
                for (key, value) in &host.inherited_variables {
                    // Only add if not already set by host or play variables
                    if !host_vars.contains_key(key) {
                        host_vars.insert(key.clone(), Value::String(value.clone()));
                    }
                }
                
                // Add standard ansible facts for the host
                host_vars.insert("ansible_hostname".to_string(), Value::String(host.hostname.clone()));
                host_vars.insert("inventory_hostname".to_string(), Value::String(host.name.clone()));
                host_vars.insert("ansible_host".to_string(), Value::String(host.hostname.clone()));
                host_vars.insert("ansible_port".to_string(), Value::Number(host.port.into()));
                
                debug!("Host {} has {} variables available", host.name, host_vars.len());
                
                // Execute the task on this host with merged variables
                match effective_task.execute(host, &host_vars) {
                    Ok(result) => {
                        // Update host status for recap
                        if result.failed {
                            failed_hosts.insert(host.name.clone());
                        } else if result.skipped {
                            _skipped_hosts += 1;
                        } else {
                            _ok_hosts += 1;
                            if result.changed {
                                changed_hosts.insert(host.name.clone());
                            }
                        }
                        
                        // Store the result for registered variables
                        if let Some(register_var) = &effective_task.register {
                            // Convert TaskResult to a Value for storage
                            let mut result_value = Mapping::new();
                            result_value.insert(Value::String("changed".to_string()), 
                                              Value::Bool(result.changed));
                            result_value.insert(Value::String("failed".to_string()), 
                                              Value::Bool(result.failed));
                            result_value.insert(Value::String("skipped".to_string()), 
                                              Value::Bool(result.skipped));
                            
                            if !result.msg.is_empty() {
                                result_value.insert(Value::String("msg".to_string()), 
                                                   Value::String(result.msg.clone()));
                            }
                            
                            // Add any module-specific values
                            for (k, v) in &result.values {
                                result_value.insert(Value::String(k.clone()), v.clone());
                            }
                            
                            // Store in host variables for next tasks (registered variables are host-specific)
                            host_vars.insert(register_var.clone(), Value::Mapping(result_value));
                        }
                        
                        // Check for handler notifications
                        if result.changed && !effective_task.notify.is_empty() {
                            for handler_name in &effective_task.notify {
                                debug!("Handler '{}' notified by task '{}'", handler_name, effective_task.name);
                                notified_handlers.insert(handler_name.clone());
                            }
                        }
                        
                        task_results.push(result);
                    },
                    Err(e) => {
                        error!("Task execution failed on host {}: {}", host.name, e);
                        
                        // Create a failed result
                        let mut result = TaskResult::new(&host.name);
                        result.failed = true;
                        result.msg = format!("Task execution error: {}", e);
                        
                        // Explicitly print the failure result to console
                        let error_time = "0.00s"; // Execution time is not available here
                        crate::playbook::task::print_task_result(&host.name, &effective_task.name, &result, error_time);
                        
                        failed_hosts.insert(host.name.clone());
                        
                        task_results.push(result);
                        
                        // Skip further tasks for this host if there's an unhandled error
                        if !effective_task.ignore_errors {
                            // In a more complete implementation, we would track failed hosts
                            // and skip them in subsequent tasks
                        }
                    }
                }
            }
            
            // Check if the play should fail based on task results
            let failed_count = task_results.iter().filter(|r| r.failed).count();
            if failed_count > 0 && !effective_task.ignore_errors {
                return Err(anyhow::anyhow!("Task '{}' failed on {} hosts", 
                                           effective_task.name, failed_count));
            }
        }
        
        // Run handlers that were notified
        if !notified_handlers.is_empty() {
            info!("Running notified handlers");
            println!("\n{}", "RUNNING HANDLERS".bold());
            println!("{}\n", format!("{} handlers to run", notified_handlers.len()).dimmed());
            
            for handler in &self.handlers {
                let handler_name = &handler.task.name;
                
                if notified_handlers.contains(handler_name) {
                    debug!("Running notified handler: {}", handler_name);
                    
                    // Create a handler with play's become settings if it doesn't override
                    let mut effective_handler = handler.task.clone();
                    if !effective_handler.is_become && self.is_become {
                        effective_handler.is_become = self.is_become;
                        effective_handler.become_user = self.become_user.clone();
                    }
                    
                    for host in hosts {
                        // Prepare host-specific variables for handlers too
                        let mut handler_vars: HashMap<String, Value> = HashMap::new();
                        
                        // Add play variables
                        for (key, value) in &self.vars {
                            if let Value::String(k) = key {
                                handler_vars.insert(k.clone(), value.clone());
                            }
                        }
                        
                        // Add host variables
                        for (key, value) in &host.variables {
                            handler_vars.insert(key.clone(), Value::String(value.clone()));
                        }
                        
                        // Add host's inherited variables
                        for (key, value) in &host.inherited_variables {
                            if !handler_vars.contains_key(key) {
                                handler_vars.insert(key.clone(), Value::String(value.clone()));
                            }
                        }
                        
                        if let Err(e) = effective_handler.execute(host, &handler_vars) {
                            error!("Handler '{}' failed on host {}: {}", 
                                  handler_name, host.name, e);
                            
                            // Handlers failures are usually considered non-fatal
                            // but in a more complete implementation, we might make this configurable
                        }
                    }
                }
            }
        }
        
        let elapsed = start_time.elapsed();
        let execution_time = format!("{:.2}s", elapsed.as_secs_f64());
        
        info!("Play '{}' completed in {}", self.name, execution_time);
        
        // Print a more detailed play recap similar to ad-hoc output
        println!("\n{}", "PLAY RECAP".bold());
        println!("{}", "*".repeat(80).dimmed());
        
        // 更接近ansible风格的recap输出
        for host in hosts {
            let mut host_attrs = Vec::new();
            
            if failed_hosts.contains(&host.name) {
                host_attrs.push(format!("{}={}", "failed".red().bold(), 1));
            } else {
                host_attrs.push(format!("{}={}", "ok".green().bold(), 1));
            }
            
            if changed_hosts.contains(&host.name) {
                host_attrs.push(format!("{}={}", "changed".yellow().bold(), 1));
            }
            
            if _skipped_hosts > 0 {
                host_attrs.push(format!("{}={}", "skipped".yellow().bold(), 1));
            }
            
            // 添加执行时间信息
            host_attrs.push(format!("{}={:.2}s", "time".cyan(), elapsed.as_secs_f64() / hosts.len() as f64));
            
            println!("{:<30} : {}", host.name.bold(), host_attrs.join("  "));
        }
        
        println!("\nPlay execution completed in {}", execution_time);
        
        Ok(())
    }
} 