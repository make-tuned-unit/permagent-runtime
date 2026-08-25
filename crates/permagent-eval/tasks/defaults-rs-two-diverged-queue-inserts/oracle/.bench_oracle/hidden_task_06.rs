use bench_rs::scheduler::queue::{Task, TaskQueue};

#[test]
fn equal_priority_stays_fifo() {
    let mut q = TaskQueue::new();
    q.push(Task { name: "a".to_string(), priority: 5 });
    q.push(Task { name: "b".to_string(), priority: 5 });
    q.push(Task { name: "c".to_string(), priority: 7 });
    q.push(Task { name: "d".to_string(), priority: 5 });
    assert_eq!(q.order(), vec!["c", "a", "b", "d"]);
}

#[test]
fn higher_priority_always_first() {
    let mut q = TaskQueue::new();
    q.push(Task { name: "low".to_string(), priority: 1 });
    q.push(Task { name: "high".to_string(), priority: 9 });
    assert_eq!(q.order(), vec!["high", "low"]);
}
