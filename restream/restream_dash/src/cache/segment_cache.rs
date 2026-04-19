use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use bytes::Bytes;

const TTL: Duration = Duration::from_secs(45);

struct Entry {
    data: Bytes,
    stored_at: Instant,
}

pub struct SegmentCache {
    store: RwLock<HashMap<String, Entry>>,
}

impl SegmentCache {
    pub fn new() -> Self {
        Self {
            store: RwLock::new(HashMap::new()),
        }
    }

    pub fn get(&self, key: &str) -> Option<Bytes> {
        let store = self.store.read().ok()?;
        let entry = store.get(key)?;
        if entry.stored_at.elapsed() > TTL {
            return None;
        }
        Some(entry.data.clone())
    }

    pub fn put(&self, key: String, data: Bytes) {
        if let Ok(mut store) = self.store.write() {
            store.insert(
                key,
                Entry {
                    data,
                    stored_at: Instant::now(),
                },
            );
        }
    }

    pub fn evict_expired(&self) {
        if let Ok(mut store) = self.store.write() {
            store.retain(|_, entry| entry.stored_at.elapsed() <= TTL);
        }
    }

    pub fn len(&self) -> usize {
        self.store.read().map(|s| s.len()).unwrap_or(0)
    }
}
