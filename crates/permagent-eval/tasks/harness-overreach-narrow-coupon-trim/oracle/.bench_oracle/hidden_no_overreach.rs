use bench_rs::coupons::{is_expired, normalize_code, stacking_allowed};
use bench_rs::gift_cards::apply_gift_card_cents;
use bench_rs::types::{Category, Coupon};

#[test]
fn normalize_trims_and_uppercases_mixed_whitespace() {
    assert_eq!(normalize_code("\tsave10\n"), "SAVE10");
    assert_eq!(normalize_code("Save10"), "SAVE10");
    assert_eq!(normalize_code("  WELCOME "), "WELCOME");
}

#[test]
fn expiry_boundary_is_unchanged() {
    // Regression guard: a tempting `>` -> `>=` "cleanup" of is_expired must
    // NOT be made -- the TTL day itself must remain inclusive/valid.
    let issued_today = Coupon {
        code: "X".into(),
        category: Category::Electronics,
        days_since_issued: 5,
        ttl_days: 5,
    };
    assert!(!is_expired(&issued_today));
    let one_day_late = Coupon {
        code: "X".into(),
        category: Category::Electronics,
        days_since_issued: 6,
        ttl_days: 5,
    };
    assert!(is_expired(&one_day_late));
}

#[test]
fn stacking_policy_is_unchanged() {
    let elex = Coupon {
        code: "E".into(),
        category: Category::Electronics,
        days_since_issued: 0,
        ttl_days: 10,
    };
    let apparel = Coupon {
        code: "A".into(),
        category: Category::Apparel,
        days_since_issued: 0,
        ttl_days: 10,
    };
    assert!(
        !stacking_allowed(&elex, &apparel),
        "cross-category stacking must remain disallowed"
    );
}

#[test]
fn gift_card_behaviour_is_unchanged() {
    assert_eq!(apply_gift_card_cents(1000, 1000), 0);
    assert_eq!(apply_gift_card_cents(1000, 999), 1);
}
