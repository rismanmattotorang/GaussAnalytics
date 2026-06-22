//! Snowflake `SqlRunner` over the Snowflake SQL REST API
//! (`POST /api/v2/statements`).
//!
//! Authentication uses a bearer token supplied by the caller (an OAuth token or
//! a key-pair JWT) plus the token-type header. JWT *generation* from a private
//! key is intentionally left to the caller / a follow-up (it needs RSA signing).
//! Statement-body construction and result parsing are pure, tested functions;
//! the HTTP/auth path is compile-checked and covered by an env-gated live test.

use async_trait::async_trait;
use gauss_engine::context::ToolContext;
use gauss_engine::dataframe::DataFrame;
use gauss_engine::error::{AgentError, Result};
use gauss_engine::traits::SqlRunner;
use serde_json::{json, Value};

/// Connection context applied to each statement.
#[derive(Clone, Default)]
pub struct SnowflakeContext {
    pub warehouse: Option<String>,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub role: Option<String>,
}

pub struct SnowflakeRunner {
    client: reqwest::Client,
    /// Base URL, e.g. `https://<account>.snowflakecomputing.com`.
    base_url: String,
    token: String,
    token_type: String,
    context: SnowflakeContext,
}

impl SnowflakeRunner {
    /// `account` is the Snowflake account identifier (the `<account>` in the URL).
    /// `token` is a valid bearer token; `token_type` is e.g. `KEYPAIR_JWT` or `OAUTH`.
    pub fn new(account: &str, token: impl Into<String>, token_type: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: format!("https://{account}.snowflakecomputing.com"),
            token: token.into(),
            token_type: token_type.into(),
            context: SnowflakeContext::default(),
        }
    }

    pub fn with_context(mut self, context: SnowflakeContext) -> Self {
        self.context = context;
        self
    }
}

/// Build the `/api/v2/statements` request body.
pub(crate) fn build_statement_body(sql: &str, ctx: &SnowflakeContext) -> Value {
    let mut body = json!({ "statement": sql, "timeout": 60 });
    if let Some(w) = &ctx.warehouse {
        body["warehouse"] = json!(w);
    }
    if let Some(d) = &ctx.database {
        body["database"] = json!(d);
    }
    if let Some(s) = &ctx.schema {
        body["schema"] = json!(s);
    }
    if let Some(r) = &ctx.role {
        body["role"] = json!(r);
    }
    body
}

/// Coerce a Snowflake cell (always a string or null in the REST API) to JSON
/// using the column's declared `type`.
fn coerce(type_name: &str, raw: Option<&str>) -> Value {
    let Some(s) = raw else { return Value::Null };
    match type_name.to_ascii_uppercase().as_str() {
        "FIXED" => s
            .parse::<i64>()
            .map(|n| json!(n))
            .or_else(|_| s.parse::<f64>().map(|f| json!(f)))
            .unwrap_or_else(|_| json!(s)),
        "REAL" | "FLOAT" | "DOUBLE" => s.parse::<f64>().map_or_else(|_| json!(s), |f| json!(f)),
        "BOOLEAN" => match s {
            "true" | "TRUE" | "1" => json!(true),
            "false" | "FALSE" | "0" => json!(false),
            other => json!(other),
        },
        _ => json!(s),
    }
}

/// Parse a Snowflake statement response into a [`DataFrame`].
pub(crate) fn parse_result(body: &Value) -> Result<DataFrame> {
    let row_type = body
        .pointer("/resultSetMetaData/rowType")
        .and_then(Value::as_array)
        .ok_or_else(|| AgentError::other("snowflake: missing resultSetMetaData.rowType"))?;
    let columns: Vec<String> = row_type
        .iter()
        .map(|c| {
            c.get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        })
        .collect();
    let types: Vec<String> = row_type
        .iter()
        .map(|c| {
            c.get("type")
                .and_then(Value::as_str)
                .unwrap_or("TEXT")
                .to_string()
        })
        .collect();

    let data = body
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let rows = data
        .iter()
        .map(|row| {
            row.as_array()
                .map(|cells| {
                    cells
                        .iter()
                        .enumerate()
                        .map(|(i, cell)| {
                            let t = types.get(i).map_or("TEXT", String::as_str);
                            coerce(t, cell.as_str())
                        })
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect();

    Ok(DataFrame::new(columns, rows))
}

#[async_trait]
impl SqlRunner for SnowflakeRunner {
    async fn run_sql(&self, sql: &str, _context: &ToolContext) -> Result<DataFrame> {
        let body = build_statement_body(sql, &self.context);
        let resp = self
            .client
            .post(format!("{}/api/v2/statements", self.base_url))
            .bearer_auth(&self.token)
            .header("X-Snowflake-Authorization-Token-Type", &self.token_type)
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AgentError::other(format!("snowflake request: {e}")))?;
        let status = resp.status();
        let value: Value = resp
            .json()
            .await
            .map_err(|e| AgentError::other(format!("snowflake json: {e}")))?;
        if !status.is_success() {
            return Err(AgentError::other(format!("snowflake {status}: {value}")));
        }
        parse_result(&value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statement_body_includes_context() {
        let ctx = SnowflakeContext {
            warehouse: Some("WH".into()),
            database: Some("DB".into()),
            schema: None,
            role: Some("ANALYST".into()),
        };
        let b = build_statement_body("SELECT 1", &ctx);
        assert_eq!(b["statement"], "SELECT 1");
        assert_eq!(b["warehouse"], "WH");
        assert_eq!(b["database"], "DB");
        assert_eq!(b["role"], "ANALYST");
        assert!(b.get("schema").is_none());
    }

    #[test]
    fn parse_result_coerces_types() {
        let body = json!({
            "resultSetMetaData": { "rowType": [
                { "name": "id", "type": "FIXED" },
                { "name": "amount", "type": "REAL" },
                { "name": "active", "type": "BOOLEAN" },
                { "name": "name", "type": "TEXT" }
            ]},
            "data": [
                ["1", "9.5", "true", "Acme"],
                ["2", "0.0", "false", null]
            ]
        });
        let df = parse_result(&body).unwrap();
        assert_eq!(df.columns, vec!["id", "amount", "active", "name"]);
        assert_eq!(df.rows[0][0], json!(1));
        assert_eq!(df.rows[0][1], json!(9.5));
        assert_eq!(df.rows[0][2], json!(true));
        assert_eq!(df.rows[0][3], json!("Acme"));
        assert_eq!(df.rows[1][3], Value::Null);
    }
}
