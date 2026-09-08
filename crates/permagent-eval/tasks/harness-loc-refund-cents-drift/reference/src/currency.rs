/// Convert a user-entered dollar string like "12.50" into integer cents.
pub fn to_cents(dollars: f64) -> i64 {
    (dollars * 100.0).round() as i64
}

/// Apply a percentage-off discount to a cents amount, e.g.
/// `apply_percent(201, 0.01)` removes 1% of $2.01, leaving $1.99.
pub fn apply_percent(cents: i64, percent_off: f64) -> i64 {
    (cents as f64 * (1.0 - percent_off)).round() as i64
}
