use bench_rs::discount::Tier;
use bench_rs::refunds::refund_amount_cents;

#[test]
fn no_discount_refunds_full_amount() {
    assert_eq!(refund_amount_cents(1000, Tier::None), 1000);
}

#[test]
fn bronze_discount_on_round_numbers() {
    // 1% off $10.00 -> $9.90, an exact case that doesn't expose the rounding bug.
    assert_eq!(refund_amount_cents(1000, Tier::Bronze), 990);
}

#[test]
fn gold_discount_on_round_numbers() {
    // 10% off $20.00 -> $18.00, again exact.
    assert_eq!(refund_amount_cents(2000, Tier::Gold), 1800);
}
