use crate::api::handlers::parse_amount;

/// Computes the price (in cents) after subtracting a discount, both given
/// as user-entered dollar strings like "12.50".
pub fn price_after_discount(base: &str, discount: &str) -> i64 {
    let base_cents = parse_amount(base);
    let discount_cents = parse_amount(discount);
    base_cents - discount_cents
}
