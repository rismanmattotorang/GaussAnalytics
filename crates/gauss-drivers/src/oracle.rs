//! Oracle driver over **ORDS REST-Enabled SQL** (`/ords/{schema}/_/sql`).
//!
//! Oracle is not a `sqlx` backend; like the Snowflake driver, this one POSTs SQL
//! with `:n` bind variables and a `binds` array, so user values are always
//! bound, never interpolated. Oracle uses `FETCH FIRST … ROWS ONLY` paging and
//! `:n` placeholders — both produced by [`gauss_query::OracleDialect`].
//!
//! `connection_uri`:
//! `oracle://{host}/ords/{schema}?user={u}&password={p}` (credentials may also
//! come from `GAUSS_ORACLE_USER` / `GAUSS_ORACLE_PASSWORD`, or a bearer
//! `token`). Integration-stage: it compiles and is structurally faithful; the
//! `#[ignore]` live test validates it against a real ORDS endpoint.

use async_trait::async_trait;
use gauss_core::domain::FieldType;
use gauss_core::error::{CoreError, CoreResult};
use gauss_query::{CompiledQuery, SqlParam};
use serde_json::{json, Value as JsonValue};

use crate::rest::{parse_query, RestConfig};
use crate::{DiscoveredColumn, DiscoveredTable, Driver, Fingerprint, QueryResult};

pub struct OracleDriver {
    client: reqwest::Client,
    endpoint: String,
    user: Option<String>,
    password: Option<String>,
    token: Option<String>,
}

fn integ<E: std::fmt::Display>(e: E) -> CoreError {
    CoreError::Integration(e.to_string())
}

/// Map a bound parameter to its JSON value for the ORDS `binds` array. Oracle
/// has no boolean type, so booleans bind as `1`/`0`.
fn ora_value(p: &SqlParam) -> JsonValue {
    match p {
        SqlParam::Int(i) => json!(i),
        SqlParam::Float(f) => json!(f),
        SqlParam::Text(s) => json!(s),
        SqlParam::Bool(b) => json!(if *b { 1 } else { 0 }),
        SqlParam::Null => JsonValue::Null,
    }
}

fn classify(t: &str) -> FieldType {
    let t = t.to_ascii_uppercase();
    if t.contains("NUMBER") || t.contains("INT") || t == "FLOAT" || t.contains("DEC") {
        // ORDS reports NUMBER for both integers and decimals; treat plain
        // NUMBER as integer and the float-y names as float.
        if t == "FLOAT" || t.contains("DEC") || t.contains("DOUBLE") || t.contains("REAL") {
            FieldType::Float
        } else {
            FieldType::Integer
        }
    } else if t.contains("DATE") || t.contains("TIMESTAMP") {
        FieldType::DateTime
    } else if t.contains("CHAR") || t.contains("CLOB") || t.contains("TEXT") {
        FieldType::Text
    } else {
        FieldType::Unknown
    }
}

impl OracleDriver {
    pub fn connect(uri: &str) -> CoreResult<Self> {
        let cfg: RestConfig = parse_query(uri, "oracle://")?;
        if cfg.host.is_empty() {
            return Err(CoreError::Config(
                "oracle uri missing host/ORDS path".into(),
            ));
        }
        // `host` is `{host}/ords/{schema}`; the REST-Enabled SQL endpoint hangs
        // off it as `/_/sql`.
        let endpoint = format!("https://{}/_/sql", cfg.host);
        let client = reqwest::Client::builder().build().map_err(integ)?;
        Ok(Self {
            client,
            endpoint,
            user: cfg
                .param("user")
                .map(str::to_string)
                .or_else(|| std::env::var("GAUSS_ORACLE_USER").ok()),
            password: cfg
                .param("password")
                .map(str::to_string)
                .or_else(|| std::env::var("GAUSS_ORACLE_PASSWORD").ok()),
            token: cfg
                .param("token")
                .map(str::to_string)
                .or_else(|| std::env::var("GAUSS_ORACLE_TOKEN").ok()),
        })
    }

    async fn query(&self, sql: &str, params: &[SqlParam]) -> CoreResult<QueryResult> {
        let binds: Vec<JsonValue> = params
            .iter()
            .enumerate()
            .map(|(i, p)| json!({ "index": i + 1, "value": ora_value(p) }))
            .collect();
        let body = json!({ "statementText": sql, "binds": binds });

        let mut req = self.client.post(&self.endpoint).json(&body);
        if let Some(token) = &self.token {
            req = req.bearer_auth(token);
        } else if let Some(user) = &self.user {
            req = req.basic_auth(user, self.password.as_ref());
        }
        let resp = req
            .send()
            .await
            .map_err(integ)?
            .error_for_status()
            .map_err(integ)?;
        let v: JsonValue = resp.json().await.map_err(integ)?;

        // ORDS returns `items[0].resultSet.{metadata,items}`; rows are objects
        // keyed by column name, which we project into column order.
        let result_set = &v["items"][0]["resultSet"];
        let columns: Vec<String> = result_set["metadata"]
            .as_array()
            .map(|m| {
                m.iter()
                    .map(|c| c["columnName"].as_str().unwrap_or_default().to_string())
                    .collect()
            })
            .unwrap_or_default();
        let rows = result_set["items"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|r| {
                        columns
                            .iter()
                            .map(|c| r.get(c).cloned().unwrap_or(JsonValue::Null))
                            .collect()
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(QueryResult { columns, rows })
    }
}

#[async_trait]
impl Driver for OracleDriver {
    async fn run(&self, query: &CompiledQuery) -> CoreResult<QueryResult> {
        self.query(&query.sql, &query.params).await
    }

    async fn sync_schema(&self) -> CoreResult<Vec<DiscoveredTable>> {
        // The current schema's tables and columns, in definition order.
        let res = self
            .query(
                "SELECT table_name, column_name, data_type FROM user_tab_columns \
                 ORDER BY table_name, column_id",
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

    #[test]
    fn parses_ords_endpoint_and_credentials() {
        let d =
            OracleDriver::connect("oracle://db.example.com/ords/sales?user=app&password=secret")
                .unwrap();
        assert_eq!(d.endpoint, "https://db.example.com/ords/sales/_/sql");
        assert_eq!(d.user.as_deref(), Some("app"));
        assert_eq!(d.password.as_deref(), Some("secret"));
    }

    #[test]
    fn booleans_bind_as_numbers() {
        assert_eq!(ora_value(&SqlParam::Bool(true)), json!(1));
        assert_eq!(ora_value(&SqlParam::Bool(false)), json!(0));
    }

    #[test]
    fn classifies_oracle_types() {
        assert_eq!(classify("NUMBER"), FieldType::Integer);
        assert_eq!(classify("FLOAT"), FieldType::Float);
        assert_eq!(classify("VARCHAR2"), FieldType::Text);
        assert_eq!(classify("TIMESTAMP(6)"), FieldType::DateTime);
    }

    #[tokio::test]
    #[ignore]
    async fn oracle_run_smoke() {
        let uri = std::env::var("GAUSS_TEST_ORACLE_URI").expect("GAUSS_TEST_ORACLE_URI");
        let d = OracleDriver::connect(&uri).unwrap();
        let compiled = gauss_query::compile(
            &gauss_core::gql::Query::new("DUAL"),
            &gauss_query::OracleDialect,
        )
        .unwrap();
        let _ = d.run(&compiled).await.unwrap();
    }
}
