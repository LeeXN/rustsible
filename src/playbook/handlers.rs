use crate::playbook::Task;

/// Handler structure (basically a named task to be triggered by notifications)
#[derive(Debug, Clone)]
pub struct Handler {
    pub task: Task,
    pub listen: Vec<String>,
    pub hosts: Option<std::collections::HashSet<String>>,
    pub cached_name: Option<String>,
}
