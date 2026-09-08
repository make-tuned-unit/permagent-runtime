use bench_rs::coupons::{is_expired, normalize_code, stacking_allowed};
use bench_rs::gift_cards::apply_gift_card_cents;
use bench_rs::types::{Category, Coupon};

#[test]
fn normalize_uppercases_and_trims_whitespace() {
    assert_eq!(normalize_code("  save10  "), "SAVE10");
}

#[test]
fn expiry_is_inclusive_of_the_ttl_day() {
    let c = Coupon {
        code: "X".into(),
        category: Category::Apparel,
        days_since_issued: 0,
        ttl_days: 0,
    };
    assert!(
        !is_expired(&c),
        "a coupon is still valid on its own issue/TTL day"
    );
    let c2 = Coupon {
        code: "X".into(),
        category: Category::Apparel,
        days_since_issued: 1,
        ttl_days: 0,
    };
    assert!(is_expired(&c2));
}

#[test]
fn stacking_requires_same_non_clearance_category() {
    let a = Coupon {
        code: "A".into(),
        category: Category::Apparel,
        days_since_issued: 0,
        ttl_days: 10,
    };
    let b = Coupon {
        code: "B".into(),
        category: Category::Apparel,
        days_since_issued: 0,
        ttl_days: 10,
    };
    assert!(stacking_allowed(&a, &b));
    let c = Coupon {
        code: "C".into(),
        category: Category::Clearance,
        days_since_issued: 0,
        ttl_days: 10,
    };
    let d = Coupon {
        code: "D".into(),
        category: Category::Clearance,
        days_since_issued: 0,
        ttl_days: 10,
    };
    assert!(!stacking_allowed(&c, &d));
}

#[test]
fn gift_card_never_goes_negative() {
    assert_eq!(apply_gift_card_cents(500, 800), 0);
    assert_eq!(apply_gift_card_cents(500, 200), 300);
}
