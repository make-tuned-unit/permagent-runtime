#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub name: String,
    pub priority: u8,
}

#[derive(Debug, Default)]
pub struct TaskQueue {
    items: Vec<Task>,
}

impl TaskQueue {
    pub fn new() -> Self {
        TaskQueue { items: Vec::new() }
    }

    /// Inserts a task, keeping the queue sorted by descending priority.
    /// Equal-priority tasks should stay in the order they were pushed.
    pub fn push(&mut self, task: Task) {
        let pos = self.items.iter().position(|t| t.priority <= task.priority);
        match pos {
            Some(i) => self.items.insert(i, task),
            None => self.items.push(task),
        }
    }

    /// Same idea as `push`, written independently for the batch-import path.
    /// Kept in sync by hand -- which is exactly the problem.
    pub fn push_stable(&mut self, task: Task) {
        let pos = self.items.iter().position(|t| t.priority < task.priority);
        match pos {
            Some(i) => self.items.insert(i, task),
            None => self.items.push(task),
        }
    }

    pub fn order(&self) -> Vec<String> {
        self.items.iter().map(|t| t.name.clone()).collect()
    }
}
