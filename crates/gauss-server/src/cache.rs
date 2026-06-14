//! A small TTL cache for query results.
//!
//! Keyed by the *compiled* query (database + SQL + bound params), so two
//! semantically identical GQL queries share a cache entry. Disabled when the
//! configured TTL is zero. This is an in-process cache; a shared cache (Redis)
//! can slot in behind the same `get`/`put` surface later.

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use gauss_drivers::QueryResult;
use gauss_query::CompiledQuery;
use uuid::Uuid;

/// An in-memory, time-bounded result cache.
pub struct ResultCache {
    ttl: Duration,
    map: RwLock<HashMap<String, (Instant, QueryResult)>>,
}

impl ResultCache {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            ttl: Duration::from_secs(ttl_secs),
            map: RwLock::new(HashMap::new()),
        }
    }

    /// Whether caching is active (TTL > 0).
    pub fn enabled(&self) -> bool {
        !self.ttl.is_zero()
    }

    /// Fetch a fresh (non-expired) entry, if present.
    pub fn get(&self, key: &str) -> Option<QueryResult> {
        if !self.enabled() {
            return None;
        }
        let guard = self.map.read().ok()?;
        let (stored_at, value) = guard.get(key)?;
        if stored_at.elapsed() < self.ttl {
            Some(value.clone())
        } else {
            None
        }
    }

    /// Store a result under `key`.
    pub fn put(&self, key: String, value: QueryResult) {
        if !self.enabled() {
            return;
        }
        if let Ok(mut guard) = self.map.write() {
            guard.insert(key, (Instant::now(), value));
        }
    }
}

/// Build a cache key from the target database and compiled query.
pub fn cache_key(database_id: Uuid, compiled: &CompiledQuery) -> String {
    format!("{database_id}|{}|{:?}", compiled.sql, compiled.params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gauss_query::SqlParam;

    fn result() -> QueryResult {
        QueryResult {
            columns: vec!["n".into()],
            rows: vec![vec![serde_json::json!(1)]],
        }
    }

    #[test]
    fn disabled_cache_never_stores() {
        let c = ResultCache::new(0);
        assert!(!c.enabled());
        c.put("k".into(), result());
        assert!(c.get("k").is_none());
    }

    #[test]
    fn stores_and_retrieves_within_ttl() {
        let c = ResultCache::new(60);
        c.put("k".into(), result());
        assert_eq!(c.get("k"), Some(result()));
        assert!(c.get("missing").is_none());
    }

    #[test]
    fn key_reflects_sql_and_params() {
        let q1 = CompiledQuery {
            sql: "SELECT 1".into(),
            params: vec![SqlParam::Int(1)],
        };
        let q2 = CompiledQuery {
            sql: "SELECT 1".into(),
            params: vec![SqlParam::Int(2)],
        };
        let id = Uuid::new_v4();
        assert_ne!(cache_key(id, &q1), cache_key(id, &q2));
    }
}
