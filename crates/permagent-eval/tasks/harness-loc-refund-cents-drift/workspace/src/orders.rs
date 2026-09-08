#[derive(Debug, Clone)]
pub struct Order {
    pub id: u64,
    pub total_cents: i64,
}

pub fn order_summary(order: &Order) -> String {
    format!(
        "order #{} total ${:.2}",
        order.id,
        order.total_cents as f64 / 100.0
    )
}
