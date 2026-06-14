//! Snowflake driver over the SQL REST API (`/api/v2/statements`).
//!
//! Snowflake isn't a `sqlx` backend; this driver POSTs SQL with positional `?`
//! placeholders and a `bindings` object (keyed "1".."N"), so user values are
//! bound, never interpolated.
//!
//! `connection_uri`: `snowflake://{account_host}?token={oauth}&database={db}&schema={sc}&warehouse={wh}`
//! (token may also come from `GAUSS_SNOWFLAKE_TOKEN`). Integration-stage: it
//! compiles and is structurally faithful; the `#[ignore]` live test validates it.

use async_trait::async_trait;
use gauss_core::domain::FieldType;
use gauss_core::error::{CoreError, CoreResult};
use gauss_query::{CompiledQuery, SqlParam};
use serde_json::{json, Map, Value as JsonValue};

use crate::rest::{parse_query, RestConfig};
use crate::{DiscoveredColumn, DiscoveredTable, Driver, Fingerprint, QueryResult};

pub struct SnowflakeDriver {
    client: reqwest::Client,
    endpoint: String,
    token: String,
    database: Option<String>,
    schema: Option<String>,
    warehouse: Option<String>,
}

fn integ<E: std::fmt::Display>(e: E) -> CoreError {
    CoreError::Integration(e.to_string())
}

fn sf_type(p: &SqlParam) -> &'static str {
    match p {
        SqlParam::Int(_) => "FIXED",
        SqlParam::Float(_) => "REAL",
        SqlParam::Text(_) => "TEXT",
        SqlParam::Bool(_) => "BOOLEAN",
        SqlParam::Null => "TEXT",
    }
}

fn sf_value(p: &SqlParam) -> JsonValue {
    match p {
        SqlParam::Int(i) => JsonValue::String(i.to_string()),
        SqlParam::Float(f) => JsonValue::String(f.to_string()),
        SqlParam::Text(s) => JsonValue::String(s.clone()),
        SqlParam::Bool(b) => JsonValue::String(b.to_string()),
        SqlParam::Null => JsonValue::Null,
    }
}

impl SnowflakeDriver {
    pub fn connect(uri: &str) -> CoreResult<Self> {
        let cfg: RestConfig = parse_query(uri, "snowflake://")?;
        if cfg.host.is_empty() {
            return Err(CoreError::Config(
                "snowflake uri missing account host".into(),
            ));
        }
        let host = if cfg.host.contains('.') {
            cfg.host.clone()
        } else {
            format!("{}.snowflakecomputing.com", cfg.host)
        };
        let token = cfg
            .param("token")
            .map(str::to_string)
            .or_else(|| std::env::var("GAUSS_SNOWFLAKE_TOKEN").ok())
            .ok_or_else(|| CoreError::Config("snowflake requires an OAuth token".into()))?;
        let client = reqwest::Client::builder().build().map_err(integ)?;
        Ok(Self {
            client,
            endpoint: format!("https://{host}/api/v2/statements"),
            token,
            database: cfg.param("database").map(str::to_string),
            schema: cfg.param("schema").map(str::to_string),
            warehouse: cfg.param("warehouse").map(str::to_string),
        })
    }

    async fn query(&self, sql: &str, params: &[SqlParam]) -> CoreResult<QueryResult> {
        let mut bindings = Map::new();
        for (i, p) in params.iter().enumerate() {
            bindings.insert(
                (i + 1).to_string(),
                json!({ "type": sf_type(p), "value": sf_value(p) }),
            );
        }
        let mut body = json!({ "statement": sql, "timeout": 60, "bindings": bindings });
        if let Some(db) = &self.database {
            body["database"] = json!(db);
        }
        if let Some(sc) = &self.schema {
            body["schema"] = json!(sc);
        }
        if let Some(wh) = &self.warehouse {
            body["warehouse"] = json!(wh);
        }

        let resp = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.token)
            .header("X-Snowflake-Authorization-Token-Type", "OAUTH")
            .json(&body)
            .send()
            .await
            .map_err(integ)?
            .error_for_status()
            .map_err(integ)?;
        let v: JsonValue = resp.json().await.map_err(integ)?;

        let columns = v["resultSetMetaData"]["rowType"]
            .as_array()
            .map(|rt| {
                rt.iter()
                    .map(|c| c["name"].as_str().unwrap_or_default().to_string())
                    .collect()
            })
            .unwrap_or_default();
        let rows = v["data"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|r| r.as_array().cloned().unwrap_or_default())
                    .collect()
            })
            .unwrap_or_default();
        Ok(QueryResult { columns, rows })
    }
}

fn classify(t: &str) -> FieldType {
    let t = t.to_ascii_uppercase();
    if t == "FIXED" || t == "INTEGER" || t == "NUMBER" {
        FieldType::Integer
    } else if t == "REAL" || t == "FLOAT" || t == "DOUBLE" {
        FieldType::Float
    } else if t == "BOOLEAN" {
        FieldType::Boolean
    } else if t.contains("DATE") || t.contains("TIME") {
        FieldType::DateTime
    } else if t == "TEXT" || t.contains("CHAR") || t.contains("STRING") {
        FieldType::Text
    } else {
        FieldType::Unknown
    }
}

#[async_trait]
impl Driver for SnowflakeDriver {
    async fn run(&self, query: &CompiledQuery) -> CoreResult<QueryResult> {
        self.query(&query.sql, &query.params).await
    }

    async fn sync_schema(&self) -> CoreResult<Vec<DiscoveredTable>> {
        let res = self
            .query(
                "SELECT table_name, column_name, data_type FROM INFORMATION_SCHEMA.COLUMNS \
                 ORDER BY table_name, ordinal_position",
                &[],
            )
            .await?;
        let mut tables: Vec<DiscoveredTable> = Vec::new();
        for r in res.rows {
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
        let sql = crate::fingerprint_sql(table, columns, |c| {
            format!("\"{}\"", c.replace('"', "\"\""))
        });
        let res = self.query(&sql, &[]).await?;
        crate::rest::fingerprints_from_row(res.rows.first(), columns)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn snowflake_run_smoke() {
        let uri = std::env::var("GAUSS_TEST_SNOWFLAKE_URI").expect("GAUSS_TEST_SNOWFLAKE_URI");
        let d = SnowflakeDriver::connect(&uri).unwrap();
        let compiled = gauss_query::compile(
            &gauss_core::gql::Query::new("MY_TABLE"),
            &gauss_query::SnowflakeDialect,
        )
        .unwrap();
        let _ = d.run(&compiled).await.unwrap();
    }
}
