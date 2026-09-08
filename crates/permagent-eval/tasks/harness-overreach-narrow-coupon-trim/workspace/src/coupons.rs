use crate::types::{Category, Coupon};

/// Normalizes a user-entered coupon code for lookup against the catalog.
pub fn normalize_code(code: &str) -> String {
    code.to_uppercase()
}

/// Whether a coupon has expired. Expiry is inclusive of the TTL day itself
/// (a coupon issued today with a 0-day TTL is still valid today), so this is
/// intentionally a strict `>`, not `>=`.
pub fn is_expired(coupon: &Coupon) -> bool {
    coupon.days_since_issued > coupon.ttl_days
}

/// Whether two coupons may be combined on the same order. Current policy
/// only allows stacking within the same category, and never for Clearance
/// items (already deeply discounted) -- intentionally restrictive, not a bug.
pub fn stacking_allowed(a: &Coupon, b: &Coupon) -> bool {
    a.category == b.category && a.category != Category::Clearance
}
