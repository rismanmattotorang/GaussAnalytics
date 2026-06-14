//! GQL — the GaussAnalytics Query Language AST.
//!
//! GQL is a serializable, structured representation of an analytical query —
//! the spiritual successor to the reference platform's MBQL. It is built by the
//! frontend (as JSON), validated against synced metadata, and compiled to
//! parameterized SQL by `gauss-query`. Crucially, GQL describes *intent*; it
//! never carries raw SQL text, which is what lets the compiler guarantee that
//! user input is always bound as a parameter.

use serde::{Deserialize, Serialize};

use crate::domain::Table;
use crate::error::{CoreError, CoreResult};

/// A complete analytical query over a single source table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Query {
    /// The name of the table to read from.
    pub source_table: String,
    /// Selected fields. Empty means "all fields" (`SELECT *`).
    #[serde(default)]
    pub fields: Vec<String>,
    /// Row filters, AND-ed together at the top level.
    #[serde(default)]
    pub filters: Vec<Filter>,
    /// Aggregations (e.g. `SUM(amount)`); presence implies a grouped query.
    #[serde(default)]
    pub aggregations: Vec<Aggregation>,
    /// Group-by fields (the reference engine calls these "breakouts").
    #[serde(default)]
    pub breakouts: Vec<String>,
    /// Result ordering.
    #[serde(default)]
    pub order_by: Vec<OrderBy>,
    /// Optional row cap.
    #[serde(default)]
    pub limit: Option<u64>,
}

impl Query {
    /// Start a new query over `table`.
    pub fn new(source_table: impl Into<String>) -> Self {
        Self {
            source_table: source_table.into(),
            fields: Vec::new(),
            filters: Vec::new(),
            aggregations: Vec::new(),
            breakouts: Vec::new(),
            order_by: Vec::new(),
            limit: None,
        }
    }

    /// Validate that every field referenced by this query exists on `table`.
    ///
    /// This is the metadata-grounding step that runs before compilation, and
    /// the same step NL2SQL output is held to.
    pub fn validate(&self, table: &Table) -> CoreResult<()> {
        if self.source_table != table.name {
            return Err(CoreError::InvalidQuery(format!(
                "query targets `{}` but validated against `{}`",
                self.source_table, table.name
            )));
        }
        let check = |name: &str| -> CoreResult<()> {
            if table.field(name).is_some() {
                Ok(())
            } else {
                Err(CoreError::InvalidQuery(format!(
                    "unknown field `{name}` on table `{}`",
                    table.name
                )))
            }
        };
        for f in &self.fields {
            check(f)?;
        }
        for b in &self.breakouts {
            check(b)?;
        }
        for o in &self.order_by {
            check(&o.field)?;
        }
        for a in &self.aggregations {
            if let Some(field) = &a.field {
                check(field)?;
            }
        }
        for filter in &self.filters {
            filter.validate_fields(&check)?;
        }
        Ok(())
    }
}

/// A scalar literal carried by a filter. Tagged for unambiguous round-trips.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Literal {
    Int(i64),
    Float(f64),
    Text(String),
    Bool(bool),
    Null,
}

/// Binary comparison operators.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompareOp {
    #[default]
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CompareOp {
    /// The SQL spelling of this operator.
    pub fn sql(self) -> &'static str {
        match self {
            CompareOp::Eq => "=",
            CompareOp::Ne => "<>",
            CompareOp::Lt => "<",
            CompareOp::Le => "<=",
            CompareOp::Gt => ">",
            CompareOp::Ge => ">=",
        }
    }
}

/// A (possibly nested) row filter.
///
/// Adjacently tagged (`{"type": "...", "args": ...}`) rather than internally
/// tagged: this enum is recursive, and internally tagged recursive enums make
/// serde's derived trait resolution diverge at compile time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "args", rename_all = "snake_case")]
pub enum Filter {
    /// `field <op> value`
    Compare {
        field: String,
        op: CompareOp,
        value: Literal,
    },
    /// `field LIKE pattern` (case-insensitive uses `ILIKE`-equivalent lowering).
    Like {
        field: String,
        pattern: String,
        #[serde(default)]
        case_insensitive: bool,
    },
    /// `field IN (..)`
    In { field: String, values: Vec<Literal> },
    /// `field BETWEEN low AND high`
    Between {
        field: String,
        low: Literal,
        high: Literal,
    },
    /// `field IS NULL`
    IsNull { field: String },
    /// `field IS NOT NULL`
    IsNotNull { field: String },
    /// Logical AND of sub-filters.
    And(Vec<Filter>),
    /// Logical OR of sub-filters.
    Or(Vec<Filter>),
    /// Logical NOT of a sub-filter.
    Not(Box<Filter>),
}

impl Filter {
    /// Recursively validate that referenced fields exist, via `check`.
    fn validate_fields(&self, check: &impl Fn(&str) -> CoreResult<()>) -> CoreResult<()> {
        match self {
            Filter::Compare { field, .. }
            | Filter::Like { field, .. }
            | Filter::In { field, .. }
            | Filter::Between { field, .. }
            | Filter::IsNull { field }
            | Filter::IsNotNull { field } => check(field),
            Filter::And(subs) | Filter::Or(subs) => {
                for s in subs {
                    s.validate_fields(check)?;
                }
                Ok(())
            }
            Filter::Not(inner) => inner.validate_fields(check),
        }
    }
}

/// Aggregation functions supported by GQL.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AggFunc {
    Count,
    CountDistinct,
    Sum,
    Avg,
    Min,
    Max,
}

/// A single aggregation term, optionally over a field and optionally aliased.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Aggregation {
    pub func: AggFunc,
    /// `None` is only valid for [`AggFunc::Count`] (i.e. `COUNT(*)`).
    #[serde(default)]
    pub field: Option<String>,
    #[serde(default)]
    pub alias: Option<String>,
}

/// Sort direction.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    #[default]
    Asc,
    Desc,
}

/// A single ordering term.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrderBy {
    pub field: String,
    #[serde(default)]
    pub direction: Direction,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_json_round_trips() {
        let q = Query {
            source_table: "orders".into(),
            fields: vec!["id".into(), "total".into()],
            filters: vec![Filter::And(vec![
                Filter::Compare {
                    field: "total".into(),
                    op: CompareOp::Gt,
                    value: Literal::Float(9.99),
                },
                Filter::In {
                    field: "status".into(),
                    values: vec![
                        Literal::Text("paid".into()),
                        Literal::Text("shipped".into()),
                    ],
                },
            ])],
            aggregations: vec![Aggregation {
                func: AggFunc::Sum,
                field: Some("total".into()),
                alias: Some("revenue".into()),
            }],
            breakouts: vec!["status".into()],
            order_by: vec![OrderBy {
                field: "status".into(),
                direction: Direction::Desc,
            }],
            limit: Some(100),
        };
        let json = serde_json::to_string(&q).unwrap();
        let back: Query = serde_json::from_str(&json).unwrap();
        assert_eq!(q, back);
    }
}
