use crate::discount::{self, Tier};

/// Computes the refund amount (in cents) for a returned order, after applying
/// the customer's loyalty-tier discount.
pub fn refund_amount_cents(order_total_cents: i64, tier: Tier) -> i64 {
    discount::loyalty_discount_cents(order_total_cents, tier)
}
