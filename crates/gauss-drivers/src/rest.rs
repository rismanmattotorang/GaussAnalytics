//! Small helpers shared by the REST/HTTP drivers (BigQuery, Snowflake).

use gauss_core::error::{CoreError, CoreResult};
use serde_json::Value as JsonValue;

use crate::Fingerprint;

/// A connection URI parsed into a host and `?key=value` parameters.
pub struct RestConfig {
    pub host: String,
    pub params: Vec<(String, String)>,
}

impl RestConfig {
    pub fn param(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// Parse `prefix://host?k=v&k2=v2` into a [`RestConfig`].
pub fn parse_query(uri: &str, prefix: &str) -> CoreResult<RestConfig> {
    let rest = uri
        .strip_prefix(prefix)
        .ok_or_else(|| CoreError::Config(format!("connection uri must start with {prefix}")))?;
    let mut parts = rest.splitn(2, '?');
    let host = parts.next().unwrap_or("").trim_end_matches('/').to_string();
    let mut params = Vec::new();
    if let Some(q) = parts.next() {
        for kv in q.split('&').filter(|s| !s.is_empty()) {
            let mut it = kv.splitn(2, '=');
            let k = it.next().unwrap_or("").to_string();
            let v = it.next().unwrap_or("").to_string();
            params.push((k, v));
        }
    }
    Ok(RestConfig { host, params })
}

fn as_i64(v: &JsonValue) -> i64 {
    v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(0)
}

/// Decode a fingerprint row (`COUNT(*), [nonnull, distinct]*`) returned by a
/// REST engine as an array of (possibly string-encoded) numbers.
pub fn fingerprints_from_row(
    row: Option<&Vec<JsonValue>>,
    columns: &[String],
) -> CoreResult<Vec<(String, Fingerprint)>> {
    let row = row.ok_or_else(|| CoreError::Integration("empty fingerprint result".into()))?;
    let total = row.first().map(as_i64).unwrap_or(0);
    let mut out = Vec::with_capacity(columns.len());
    for (i, c) in columns.iter().enumerate() {
        let nonnull = row.get(1 + 2 * i).map(as_i64).unwrap_or(0);
        let distinct = row.get(2 + 2 * i).map(as_i64).unwrap_or(0);
        out.push((
            c.clone(),
            Fingerprint {
                total_rows: total,
                null_count: total - nonnull,
                distinct_count: distinct,
            },
        ));
    }
    Ok(out)
}
