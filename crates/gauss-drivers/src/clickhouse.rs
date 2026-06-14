//! ClickHouse data-source driver over the HTTP interface.
//!
//! ClickHouse isn't a `sqlx` backend, so this driver speaks its HTTP API
//! directly: SQL is sent in the request body with ` FORMAT JSONCompact`, and the
//! GQL-compiled `{pN:Type}` substitution parameters are supplied as
//! `param_pN=...` query-string values. This keeps GaussAnalytics' guarantee that
//! user values are bound parameters, never interpolated SQL.
//!
//! `connection_uri` is the ClickHouse HTTP endpoint, e.g.
//! `clickhouse://user:pass@host:8123/?database=analytics` (normalized to
//! `http(s)://…`). Live tests are `#[ignore]`d; set `GAUSS_TEST_CLICKHOUSE_URL`.

use async_trait::async_trait;
use gauss_core::domain::FieldType;
use gauss_core::error::{CoreError, CoreResult};
use gauss_query::{CompiledQuery, SqlParam};
use serde_json::Value as JsonValue;

use crate::{DiscoveredColumn, DiscoveredTable, Driver, Fingerprint, QueryResult};

pub struct ClickHouseDriver {
    client: reqwest::Client,
    endpoint: String,
}

fn integ<E: std::fmt::Display>(e: E) -> CoreError {
    CoreError::Integration(e.to_string())
}

/// ClickHouse expects substitution-parameter values as plain strings; it parses
/// each per the declared `{pN:Type}`.
fn ch_value(p: &SqlParam) -> String {
    match p {
        SqlParam::Int(i) => i.to_string(),
        SqlParam::Float(f) => f.to_string(),
        SqlParam::Text(s) => s.clone(),
        SqlParam::Bool(b) => if *b { "1" } else { "0" }.to_string(),
        SqlParam::Null => String::new(),
    }
}

impl ClickHouseDriver {
    pub fn connect(uri: &str) -> CoreResult<Self> {
        let endpoint = if let Some(rest) = uri.strip_prefix("clickhouse://") {
            format!("http://{rest}")
        } else {
            uri.to_string()
        };
        let client = reqwest::Client::builder()
            .build()
            .map_err(|e| CoreError::Integration(format!("clickhouse client init: {e}")))?;
        Ok(Self { client, endpoint })
    }

    /// POST a query in `JSONCompact` format and return `(columns, data)`.
    async fn query_compact(
        &self,
        sql: &str,
        params: &[SqlParam],
    ) -> CoreResult<(Vec<String>, Vec<Vec<JsonValue>>)> {
        let body = format!("{sql} FORMAT JSONCompact");
        let query: Vec<(String, String)> = params
            .iter()
            .enumerate()
            .map(|(i, p)| (format!("param_p{}", i + 1), ch_value(p)))
            .collect();

        let resp = self
            .client
            .post(&self.endpoint)
            .query(&query)
            .body(body)
            .send()
            .await
            .map_err(integ)?
            .error_for_status()
            .map_err(integ)?;
        let v: JsonValue = resp.json().await.map_err(integ)?;

        let columns = v["meta"]
            .as_array()
            .map(|m| {
                m.iter()
                    .map(|c| c["name"].as_str().unwrap_or_default().to_string())
                    .collect()
            })
            .unwrap_or_default();
        let data = v["data"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|r| r.as_array().cloned().unwrap_or_default())
                    .collect()
            })
            .unwrap_or_default();
        Ok((columns, data))
    }
}

fn classify(ch_type: &str) -> FieldType {
    let t = ch_type;
    if t.contains("Int") || t.contains("UInt") {
        FieldType::Integer
    } else if t.contains("Float") || t.contains("Decimal") {
        FieldType::Float
    } else if t.contains("Bool") {
        FieldType::Boolean
    } else if t.contains("Date") || t.contains("Time") {
        FieldType::DateTime
    } else if t.contains("String") || t.contains("FixedString") || t.contains("UUID") {
        FieldType::Text
    } else {
        FieldType::Unknown
    }
}

#[async_trait]
impl Driver for ClickHouseDriver {
    async fn run(&self, query: &CompiledQuery) -> CoreResult<QueryResult> {
        let (columns, rows) = self.query_compact(&query.sql, &query.params).await?;
        Ok(QueryResult { columns, rows })
    }

    async fn sync_schema(&self) -> CoreResult<Vec<DiscoveredTable>> {
        let (_c, rows) = self
            .query_compact(
                "SELECT table, name, type FROM system.columns \
                 WHERE database = currentDatabase() ORDER BY table, position",
                &[],
            )
            .await?;
        let mut tables: Vec<DiscoveredTable> = Vec::new();
        for r in rows {
            let table = r
                .first()
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let col = r
                .get(1)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let ty = r.get(2).and_then(|v| v.as_str()).unwrap_or_default();
            let column = DiscoveredColumn {
                name: col,
                field_type: classify(ty),
            };
            match tables.iter_mut().find(|t| t.name == table) {
                Some(t) => t.columns.push(column),
                None => tables.push(DiscoveredTable {
                    name: table,
                    columns: vec![column],
                }),
            }
        }
        Ok(tables)
    }

    async fn fingerprint(
        &self,
        table: &str,
        columns: &[String],
    ) -> CoreResult<Vec<(String, Fingerprint)>> {
        if columns.is_empty() {
            return Ok(Vec::new());
        }
        let sql = crate::fingerprint_sql(table, columns, |c| format!("`{}`", c.replace('`', "``")));
        let (_c, rows) = self.query_compact(&sql, &[]).await?;
        let row = rows
            .first()
            .ok_or_else(|| integ("empty fingerprint result"))?;
        let as_i64 = |v: &JsonValue| -> i64 {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                .unwrap_or(0)
        };
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn clickhouse_run_smoke() {
        let uri = std::env::var("GAUSS_TEST_CLICKHOUSE_URL").expect("GAUSS_TEST_CLICKHOUSE_URL");
        let d = ClickHouseDriver::connect(&uri).unwrap();
        let compiled = gauss_query::compile(
            &gauss_core::gql::Query::new("system.one"),
            &gauss_query::ClickHouseDialect,
        )
        .unwrap();
        let _ = d.run(&compiled).await.unwrap();
    }
}
