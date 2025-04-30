use crate::playbook::Task;

/// Handler structure (basically a named task to be triggered by notifications)
#[derive(Debug, Clone)]
pub struct Handler {
    pub task: Task,
} 