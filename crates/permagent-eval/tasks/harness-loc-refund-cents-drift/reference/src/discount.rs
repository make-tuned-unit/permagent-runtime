use crate::currency;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    None,
    Bronze,
    Silver,
    Gold,
}

impl Tier {
    pub fn percent_off(self) -> f64 {
        match self {
            Tier::None => 0.0,
            Tier::Bronze => 0.01,
            Tier::Silver => 0.05,
            Tier::Gold => 0.10,
        }
    }
}

/// The loyalty-tier discount applied to a refund, in cents.
pub fn loyalty_discount_cents(order_total_cents: i64, tier: Tier) -> i64 {
    currency::apply_percent(order_total_cents, tier.percent_off())
}
