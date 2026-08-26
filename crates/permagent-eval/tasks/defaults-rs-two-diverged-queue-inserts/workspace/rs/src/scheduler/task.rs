#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskMeta {
    pub id: u32,
    pub label: String,
}

pub fn make_task_meta(id: u32, label: &str) -> TaskMeta {
    TaskMeta {
        id,
        label: label.to_string(),
    }
}
