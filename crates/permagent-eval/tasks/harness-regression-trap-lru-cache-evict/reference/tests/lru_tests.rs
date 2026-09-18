use bench_rs::cache::RecentCache;

#[test]
fn new_cache_is_empty() {
    let c: RecentCache<String, i32> = RecentCache::new(2);
    assert_eq!(c.len(), 0);
    assert!(c.is_empty());
}

#[test]
fn insert_and_get_roundtrip() {
    let mut c = RecentCache::new(2);
    c.insert("a".to_string(), 1);
    assert_eq!(c.get(&"a".to_string()), Some(&1));
}

#[test]
fn capacity_is_enforced() {
    let mut c = RecentCache::new(2);
    c.insert("a".to_string(), 1);
    c.insert("b".to_string(), 2);
    c.insert("c".to_string(), 3);
    c.insert("d".to_string(), 4);
    assert_eq!(c.len(), 2, "cache must never hold more than its capacity");
}
