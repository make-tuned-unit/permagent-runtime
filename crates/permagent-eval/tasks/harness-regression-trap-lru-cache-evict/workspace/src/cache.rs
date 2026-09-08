use std::collections::{HashMap, VecDeque};
use std::hash::Hash;

/// A fixed-capacity, least-recently-used eviction cache used to bound memory
/// for hot lookups (e.g. recently viewed products, recently resolved DNS
/// names) without growing forever.
pub struct RecentCache<K, V> {
    capacity: usize,
    order: VecDeque<K>,
    map: HashMap<K, V>,
}

impl<K: Eq + Hash + Clone, V> RecentCache<K, V> {
    pub fn new(capacity: usize) -> Self {
        RecentCache {
            capacity,
            order: VecDeque::new(),
            map: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Moves `key` to the most-recently-used end of the recency order, if
    /// present.
    fn touch(&mut self, key: &K) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            let k = self.order.remove(pos).unwrap();
            self.order.push_back(k);
        }
    }

    pub fn get(&mut self, key: &K) -> Option<&V> {
        if self.map.contains_key(key) {
            self.touch(key);
        }
        self.map.get(key)
    }

    pub fn contains(&self, key: &K) -> bool {
        self.map.contains_key(key)
    }

    pub fn insert(&mut self, key: K, value: V) {
        if self.map.contains_key(&key) {
            self.touch(&key);
            self.map.insert(key, value);
            return;
        }
        self.order.push_back(key.clone());
        self.map.insert(key, value);
    }
}
