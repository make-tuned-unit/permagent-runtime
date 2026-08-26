/// Parses a user-entered dollar amount like "12.50" into integer cents.
pub fn parse_amount(input: &str) -> i64 {
    let trimmed = input.trim();
    let value: f64 = trimmed.parse().unwrap();
    (value * 100.0).round() as i64
}

pub fn total_order_cents(amounts: &[&str]) -> i64 {
    amounts.iter().map(|a| parse_amount(a)).sum()
}
