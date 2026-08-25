pub mod queue;
pub mod task;

use queue::{Task, TaskQueue};

pub fn import_batch(target: &mut TaskQueue, tasks: Vec<Task>) {
    for t in tasks {
        target.push_stable(t);
    }
}
