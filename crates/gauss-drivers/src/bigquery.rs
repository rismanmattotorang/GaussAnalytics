//! Google BigQuery driver over the REST `jobs.query` API.
//!
//! BigQuery isn't a `sqlx` backend; this driver POSTs StandardSQL with
//! **positional `?` parameters** (`parameterMode: POSITIONAL`) so user values
//! remain bound, never interpolated.
//!
//! `connection_uri`: `bigquery://{project}?dataset={ds}&token={oauth}` — the
//! OAuth access token may also come from `GAUSS_BIGQUERY_TOKEN`. This is an
//! integration-stage driver: it compiles and is structurally faithful to the
//! API; correctness is validated by the `#[ignore]` live test
//! (`GAUSS_TEST_BIGQUERY_*`).

use async_trait::async_trait;
use gauss_core::domain::FieldType;
use gauss_core::error::{CoreError, CoreResult};
use gauss_query::{CompiledQuery, SqlParam};
use serde_json::{json, Value as JsonValue};

use crate::rest::{parse_query, RestConfig};
use crate::{DiscoveredColumn, DiscoveredTable, Driver, Fingerprint, QueryResult};

pub struct BigQueryDriver {
    client: reqwest::Client,
    project: String,
    dataset: Option<String>,
    token: String,
}

fn integ<E: std::fmt::Display>(e: E) -> CoreError {
    CoreError::Integration(e.to_string())
}

fn bq_type(p: &SqlParam) -> &'static str {
    match p {
        SqlParam::Int(_) => "INT64",
        SqlParam::Float(_) => "FLOAT64",
        SqlParam::Text(_) => "STRING",
        SqlParam::Bool(_) => "BOOL",
        SqlParam::Null => "STRING",
    }
}

fn bq_value(p: &SqlParam) -> JsonValue {
    match p {
        SqlParam::Int(i) => json!({ "value": i.to_string() }),
        SqlParam::Float(f) => json!({ "value": f.to_string() }),
        SqlParam::Text(s) => json!({ "value": s }),
        SqlParam::Bool(b) => json!({ "value": b.to_string() }),
        SqlParam::Null => json!({ "value": JsonValue::Null }),
    }
}

impl BigQueryDriver {
    pub fn connect(uri: &str) -> CoreResult<Self> {
        let cfg: RestConfig = parse_query(uri, "bigquery://")?;
        let project = cfg.host.clone();
        if project.is_empty() {
            return Err(CoreError::Config("bigquery uri missing project".into()));
        }
        let token = cfg
            .param("token")
            .map(str::to_string)
            .or_else(|| std::env::var("GAUSS_BIGQUERY_TOKEN").ok())
            .ok_or_else(|| CoreError::Config("bigquery requires an OAuth token".into()))?;
        let client = reqwest::Client::builder().build().map_err(integ)?;
        Ok(Self {
            client,
            project,
            dataset: cfg.param("dataset").map(str::to_string),
            token,
        })
    }

    async fn query(&self, sql: &str, params: &[SqlParam]) -> CoreResult<QueryResult> {
        let url = format!(
            "https://bigquery.googleapis.com/bigquery/v2/projects/{}/queries",
            self.project
        );
        let query_params: Vec<JsonValue> = params
            .iter()
            .map(|p| json!({ "parameterType": { "type": bq_type(p) }, "parameterValue": bq_value(p) }))
            .collect();
        let body = json!({
            "query": sql,
            "useLegacySql": false,
            "parameterMode": "POSITIONAL",
            "queryParameters": query_params,
        });

        let resp = self
            .client
            .post(url)
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(integ)?
            .error_for_status()
            .map_err(integ)?;
        let v: JsonValue = resp.json().await.map_err(integ)?;

        let columns = v["schema"]["fields"]
            .as_array()
            .map(|f| {
                f.iter()
                    .map(|c| c["name"].as_str().unwrap_or_default().to_string())
                    .collect()
            })
            .unwrap_or_default();
        let rows = v["rows"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|r| {
                        r["f"]
                            .as_array()
                            .map(|cells| cells.iter().map(|c| c["v"].clone()).collect())
                            .unwrap_or_default()
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(QueryResult { columns, rows })
    }
}

fn classify(t: &str) -> FieldType {
    match t {
        "INT64" | "INTEGER" => FieldType::Integer,
        "FLOAT64" | "FLOAT" | "NUMERIC" | "BIGNUMERIC" => FieldType::Float,
        "BOOL" | "BOOLEAN" => FieldType::Boolean,
        "TIMESTAMP" | "DATE" | "DATETIME" | "TIME" => FieldType::DateTime,
        "STRING" | "BYTES" => FieldType::Text,
        _ => FieldType::Unknown,
    }
}

#[async_trait]
impl Driver for BigQueryDriver {
    async fn run(&self, query: &CompiledQuery) -> CoreResult<QueryResult> {
        self.query(&query.sql, &query.params).await
    }

    async fn sync_schema(&self) -> CoreResult<Vec<DiscoveredTable>> {
        let dataset = self
            .dataset
            .as_ref()
            .ok_or_else(|| CoreError::Config("bigquery sync requires ?dataset=".into()))?;
        let sql = format!(
            "SELECT table_name, column_name, data_type FROM `{}`.{}.INFORMATION_SCHEMA.COLUMNS \
             ORDER BY table_name, ordinal_position",
            self.project, dataset
        );
        let res = self.query(&sql, &[]).await?;
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
        let sql = crate::fingerprint_sql(table, columns, |c| format!("`{}`", c.replace('`', "``")));
        let res = self.query(&sql, &[]).await?;
        crate::rest::fingerprints_from_row(res.rows.first(), columns)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn bigquery_run_smoke() {
        let uri = std::env::var("GAUSS_TEST_BIGQUERY_URI").expect("GAUSS_TEST_BIGQUERY_URI");
        let d = BigQueryDriver::connect(&uri).unwrap();
        let compiled = gauss_query::compile(
            &gauss_core::gql::Query::new("dataset.table"),
            &gauss_query::BigQueryDialect,
        )
        .unwrap();
        let _ = d.run(&compiled).await.unwrap();
    }
}
