use bench_rs::cache::RecentCache;

#[test]
fn evicts_the_least_recently_used_entry() {
    let mut c = RecentCache::new(2);
    c.insert("a", 1);
    c.insert("b", 2);
    c.insert("c", 3); // capacity 2: "a" should be evicted (least recently used)
    assert_eq!(c.len(), 2);
    assert!(
        !c.contains(&"a"),
        "least-recently-used entry must be evicted"
    );
    assert!(c.contains(&"b"));
    assert!(c.contains(&"c"));
}

#[test]
fn get_refreshes_recency_so_it_survives_eviction() {
    let mut c = RecentCache::new(2);
    c.insert("a", 1);
    c.insert("b", 2);
    c.get(&"a"); // touching "a" makes "b" the least-recently-used one
    c.insert("d", 4);
    assert!(
        c.contains(&"a"),
        "recently-accessed entries must not be evicted"
    );
    assert!(
        !c.contains(&"b"),
        "the untouched entry should be the one evicted"
    );
    assert!(c.contains(&"d"));
}

#[test]
fn reinserting_an_existing_key_updates_value_without_growing() {
    let mut c = RecentCache::new(2);
    c.insert("a", 1);
    c.insert("a", 99);
    assert_eq!(c.len(), 1);
    assert_eq!(c.get(&"a"), Some(&99));
}

#[test]
fn many_inserts_never_exceed_capacity() {
    let mut c = RecentCache::new(3);
    for i in 0..50 {
        c.insert(i, i * 10);
    }
    assert_eq!(c.len(), 3);
    // The three most recently inserted keys (47, 48, 49) must be the survivors.
    assert!(c.contains(&47));
    assert!(c.contains(&48));
    assert!(c.contains(&49));
}
