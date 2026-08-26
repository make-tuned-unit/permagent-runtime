use bench_rs::api::handlers::{parse_amount, total_order_cents};
use bench_rs::inventory::pricing::price_after_discount;

#[test]
fn parse_amount_ok() {
    assert_eq!(parse_amount("12.50"), Ok(1250));
}

#[test]
fn parse_amount_err_on_garbage() {
    assert!(parse_amount("not-a-number").is_err());
}

#[test]
fn total_order_cents_ok_and_err() {
    assert_eq!(total_order_cents(&["12.50", "3.25"]), Ok(1575));
    assert!(total_order_cents(&["12.50", "oops"]).is_err());
}

#[test]
fn price_after_discount_ok_and_err() {
    assert_eq!(price_after_discount("20.00", "5.00"), Ok(1500));
    assert!(price_after_discount("20.00", "oops").is_err());
}
