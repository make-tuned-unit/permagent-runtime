use bench_rs::inventory::stock::available_quantity;

#[test]
fn reserved_exceeds_total_returns_zero() {
    assert_eq!(available_quantity(5, 10), 0);
}

#[test]
fn normal_cases_still_work() {
    assert_eq!(available_quantity(10, 3), 7);
    assert_eq!(available_quantity(0, 0), 0);
    assert_eq!(available_quantity(4, 4), 0);
}
