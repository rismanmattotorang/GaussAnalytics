//! `gauss-query` — compile [`gauss_core::gql::Query`] into parameterized SQL.
//!
//! The compiler's contract is the platform's central SQL-injection defense:
//! it emits SQL containing **only** placeholders, paired with a typed vector of
//! bound parameters. A user-supplied literal can never become SQL text, by
//! construction — there is simply no code path that interpolates a [`Literal`]
//! into the query string. The same guarantee covers NL2SQL output, which is
//! validated and compiled through this crate before execution.

#![forbid(unsafe_code)]

pub mod dialect;

use gauss_core::error::{CoreError, CoreResult};
use gauss_core::gql::{AggFunc, Aggregation, Filter, Literal, Query};
use serde::{Deserialize, Serialize};

pub use dialect::{
    BigQueryDialect, ClickHouseDialect, Dialect, GenericDialect, MySqlDialect, PostgresDialect,
    SnowflakeDialect, SqliteDialect,
};

/// A typed parameter to be bound at execution time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum SqlParam {
    Int(i64),
    Float(f64),
    Text(String),
    Bool(bool),
    Null,
}

impl From<&Literal> for SqlParam {
    fn from(l: &Literal) -> Self {
        match l {
            Literal::Int(i) => SqlParam::Int(*i),
            Literal::Float(f) => SqlParam::Float(*f),
            Literal::Text(s) => SqlParam::Text(s.clone()),
            Literal::Bool(b) => SqlParam::Bool(*b),
            Literal::Null => SqlParam::Null,
        }
    }
}

/// The result of compiling a [`Query`]: SQL text plus its bound parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompiledQuery {
    /// SQL with placeholders only — safe to log and to send to the driver.
    pub sql: String,
    /// Parameters bound positionally to the placeholders in `sql`.
    pub params: Vec<SqlParam>,
}

/// Compile `query` to parameterized SQL for the given `dialect`.
pub fn compile(query: &Query, dialect: &dyn Dialect) -> CoreResult<CompiledQuery> {
    let mut b = Builder {
        dialect,
        params: Vec::new(),
    };
    let sql = b.build(query)?;
    Ok(CompiledQuery {
        sql,
        params: b.params,
    })
}

/// Internal compilation state: accumulates bound parameters as it renders SQL.
struct Builder<'d> {
    dialect: &'d dyn Dialect,
    params: Vec<SqlParam>,
}

impl Builder<'_> {
    /// Bind a literal and return its placeholder token.
    fn bind(&mut self, lit: &Literal) -> String {
        let param = SqlParam::from(lit);
        let placeholder = self.dialect.placeholder(self.params.len() + 1, &param);
        self.params.push(param);
        placeholder
    }

    fn ident(&self, name: &str) -> String {
        self.dialect.quote_ident(name)
    }

    fn build(&mut self, q: &Query) -> CoreResult<String> {
        let select = self.build_select(q)?;
        let mut sql = format!("SELECT {select} FROM {}", self.ident(&q.source_table));

        if !q.filters.is_empty() {
            let mut clauses = Vec::with_capacity(q.filters.len());
            for f in &q.filters {
                clauses.push(self.build_filter(f)?);
            }
            sql.push_str(" WHERE ");
            sql.push_str(&clauses.join(" AND "));
        }

        if !q.aggregations.is_empty() && !q.breakouts.is_empty() {
            let cols: Vec<String> = q.breakouts.iter().map(|c| self.ident(c)).collect();
            sql.push_str(" GROUP BY ");
            sql.push_str(&cols.join(", "));
        }

        if !q.order_by.is_empty() {
            let terms: Vec<String> = q
                .order_by
                .iter()
                .map(|o| {
                    let dir = match o.direction {
                        gauss_core::gql::Direction::Asc => "ASC",
                        gauss_core::gql::Direction::Desc => "DESC",
                    };
                    format!("{} {}", self.ident(&o.field), dir)
                })
                .collect();
            sql.push_str(" ORDER BY ");
            sql.push_str(&terms.join(", "));
        }

        if let Some(limit) = q.limit {
            // `limit` is an integer we control, so inlining is injection-safe.
            sql.push_str(&format!(" LIMIT {limit}"));
        }

        Ok(sql)
    }

    fn build_select(&mut self, q: &Query) -> CoreResult<String> {
        if !q.aggregations.is_empty() {
            let mut cols: Vec<String> = q.breakouts.iter().map(|c| self.ident(c)).collect();
            for agg in &q.aggregations {
                cols.push(self.build_aggregation(agg)?);
            }
            if cols.is_empty() {
                return Err(CoreError::Compilation(
                    "aggregate query has no breakouts and no aggregations to select".into(),
                ));
            }
            Ok(cols.join(", "))
        } else if q.fields.is_empty() {
            Ok("*".to_string())
        } else {
            let cols: Vec<String> = q.fields.iter().map(|c| self.ident(c)).collect();
            Ok(cols.join(", "))
        }
    }

    fn build_aggregation(&self, agg: &Aggregation) -> CoreResult<String> {
        let needs_field = !matches!(agg.func, AggFunc::Count);
        let expr = match (&agg.func, &agg.field) {
            (AggFunc::Count, None) => "COUNT(*)".to_string(),
            (AggFunc::Count, Some(f)) => format!("COUNT({})", self.ident(f)),
            (AggFunc::CountDistinct, Some(f)) => format!("COUNT(DISTINCT {})", self.ident(f)),
            (AggFunc::Sum, Some(f)) => format!("SUM({})", self.ident(f)),
            (AggFunc::Avg, Some(f)) => format!("AVG({})", self.ident(f)),
            (AggFunc::Min, Some(f)) => format!("MIN({})", self.ident(f)),
            (AggFunc::Max, Some(f)) => format!("MAX({})", self.ident(f)),
            _ if needs_field => {
                return Err(CoreError::Compilation(format!(
                    "aggregation {:?} requires a field",
                    agg.func
                )));
            }
            _ => unreachable!("Count is the only field-less aggregation"),
        };
        match &agg.alias {
            Some(alias) => Ok(format!("{expr} AS {}", self.ident(alias))),
            None => Ok(expr),
        }
    }

    fn build_filter(&mut self, f: &Filter) -> CoreResult<String> {
        let sql = match f {
            Filter::Compare { field, op, value } => {
                let ph = self.bind(value);
                format!("{} {} {}", self.ident(field), op.sql(), ph)
            }
            Filter::Like {
                field,
                pattern,
                case_insensitive,
            } => {
                let ph = self.bind(&Literal::Text(pattern.clone()));
                if *case_insensitive {
                    format!("LOWER({}) LIKE LOWER({})", self.ident(field), ph)
                } else {
                    format!("{} LIKE {}", self.ident(field), ph)
                }
            }
            Filter::In { field, values } => {
                if values.is_empty() {
                    // `x IN ()` is invalid SQL; an empty set matches nothing.
                    "1 = 0".to_string()
                } else {
                    let phs: Vec<String> = values.iter().map(|v| self.bind(v)).collect();
                    format!("{} IN ({})", self.ident(field), phs.join(", "))
                }
            }
            Filter::Between { field, low, high } => {
                let lo = self.bind(low);
                let hi = self.bind(high);
                format!("{} BETWEEN {} AND {}", self.ident(field), lo, hi)
            }
            Filter::IsNull { field } => format!("{} IS NULL", self.ident(field)),
            Filter::IsNotNull { field } => format!("{} IS NOT NULL", self.ident(field)),
            Filter::And(subs) => self.build_junction(subs, "AND", "1 = 1")?,
            Filter::Or(subs) => self.build_junction(subs, "OR", "1 = 0")?,
            Filter::Not(inner) => format!("(NOT ({}))", self.build_filter(inner)?),
        };
        Ok(sql)
    }

    fn build_junction(&mut self, subs: &[Filter], joiner: &str, empty: &str) -> CoreResult<String> {
        if subs.is_empty() {
            return Ok(empty.to_string());
        }
        let mut parts = Vec::with_capacity(subs.len());
        for s in subs {
            parts.push(self.build_filter(s)?);
        }
        Ok(format!("({})", parts.join(&format!(" {joiner} "))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gauss_core::gql::{CompareOp, Direction, OrderBy};

    fn pg() -> PostgresDialect {
        PostgresDialect
    }

    #[test]
    fn select_star_with_limit() {
        let mut q = Query::new("orders");
        q.limit = Some(10);
        let c = compile(&q, &pg()).unwrap();
        assert_eq!(c.sql, r#"SELECT * FROM "orders" LIMIT 10"#);
        assert!(c.params.is_empty());
    }

    #[test]
    fn user_literals_become_bound_params_not_sql_text() {
        // A classic injection attempt must end up as a *parameter*, never SQL.
        let mut q = Query::new("users");
        q.fields = vec!["id".into()];
        q.filters = vec![Filter::Compare {
            field: "email".into(),
            op: CompareOp::Eq,
            value: Literal::Text("x'; DROP TABLE users; --".into()),
        }];
        let c = compile(&q, &pg()).unwrap();
        assert_eq!(c.sql, r#"SELECT "id" FROM "users" WHERE "email" = $1"#);
        assert_eq!(
            c.params,
            vec![SqlParam::Text("x'; DROP TABLE users; --".into())]
        );
        // The dangerous string is nowhere in the SQL text.
        assert!(!c.sql.contains("DROP TABLE"));
    }

    #[test]
    fn grouped_aggregation() {
        let q = Query {
            source_table: "orders".into(),
            fields: vec![],
            filters: vec![Filter::Compare {
                field: "total".into(),
                op: CompareOp::Ge,
                value: Literal::Int(100),
            }],
            aggregations: vec![Aggregation {
                func: AggFunc::Sum,
                field: Some("total".into()),
                alias: Some("revenue".into()),
            }],
            breakouts: vec!["status".into()],
            order_by: vec![OrderBy {
                field: "status".into(),
                direction: Direction::Asc,
            }],
            limit: None,
        };
        let c = compile(&q, &pg()).unwrap();
        assert_eq!(
            c.sql,
            r#"SELECT "status", SUM("total") AS "revenue" FROM "orders" WHERE "total" >= $1 GROUP BY "status" ORDER BY "status" ASC"#
        );
        assert_eq!(c.params, vec![SqlParam::Int(100)]);
    }

    #[test]
    fn nested_and_or_filters() {
        let mut q = Query::new("events");
        q.filters = vec![Filter::Or(vec![
            Filter::Compare {
                field: "kind".into(),
                op: CompareOp::Eq,
                value: Literal::Text("click".into()),
            },
            Filter::And(vec![
                Filter::IsNotNull {
                    field: "user_id".into(),
                },
                Filter::In {
                    field: "region".into(),
                    values: vec![Literal::Text("us".into()), Literal::Text("eu".into())],
                },
            ]),
        ])];
        let c = compile(&q, &SqliteDialect).unwrap();
        assert_eq!(
            c.sql,
            r#"SELECT * FROM "events" WHERE ("kind" = ? OR ("user_id" IS NOT NULL AND "region" IN (?, ?)))"#
        );
        assert_eq!(c.params.len(), 3);
    }

    #[test]
    fn mysql_uses_backticks_and_qmark() {
        let mut q = Query::new("t");
        q.fields = vec!["a".into()];
        q.filters = vec![Filter::Compare {
            field: "a".into(),
            op: CompareOp::Lt,
            value: Literal::Int(5),
        }];
        let c = compile(&q, &MySqlDialect).unwrap();
        assert_eq!(c.sql, "SELECT `a` FROM `t` WHERE `a` < ?");
    }

    #[test]
    fn empty_in_matches_nothing() {
        let mut q = Query::new("t");
        q.filters = vec![Filter::In {
            field: "x".into(),
            values: vec![],
        }];
        let c = compile(&q, &GenericDialect).unwrap();
        assert!(c.sql.ends_with("WHERE 1 = 0"));
    }
}
