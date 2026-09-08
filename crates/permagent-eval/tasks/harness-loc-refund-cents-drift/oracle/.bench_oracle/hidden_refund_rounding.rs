use bench_rs::currency::apply_percent;
use bench_rs::discount::Tier;
use bench_rs::refunds::refund_amount_cents;

#[test]
fn bronze_discount_rounds_instead_of_truncating() {
    // $2.01 at 1% off should refund $1.99 (199 cents), not $1.98.
    assert_eq!(refund_amount_cents(201, Tier::Bronze), 199);
}

#[test]
fn silver_discount_rounds_a_fractional_case() {
    // $4.03 at 5% off = 382.85 cents -> rounds to 383, not 382.
    assert_eq!(refund_amount_cents(403, Tier::Silver), 383);
}

#[test]
fn apply_percent_rounds_half_up_cases() {
    assert_eq!(apply_percent(201, 0.01), 199);
    assert_eq!(apply_percent(1000, 0.0), 1000);
}
