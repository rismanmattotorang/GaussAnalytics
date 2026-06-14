//! Differential testing: the same GQL query, executed by the SQLite driver and
//! by an independent in-Rust reference evaluator, must yield identical rows.
//!
//! This catches divergence between the compiler/driver path and the intended
//! GQL semantics without needing multiple live database engines — it runs in
//! CI. As real Postgres/MySQL engines become available in test environments,
//! the same fixtures can be pointed at them for cross-engine differential runs.

use std::cmp::Ordering;
use std::collections::HashMap;

use gauss_core::gql::{CompareOp, Direction, Filter, Literal, OrderBy, Query};
use gauss_drivers::{Driver, SqliteDriver};
use serde_json::{json, Value as JsonValue};
use sqlx::sqlite::SqlitePoolOptions;

/// A reference cell value mirroring the fixture column types.
#[derive(Clone, Debug)]
enum Cell {
    Int(i64),
    Float(f64),
    Text(String),
    Null,
}

impl Cell {
    fn to_json(&self) -> JsonValue {
        match self {
            Cell::Int(i) => json!(i),
            Cell::Float(f) => json!(f),
            Cell::Text(s) => json!(s),
            Cell::Null => JsonValue::Null,
        }
    }
    fn as_f64(&self) -> Option<f64> {
        match self {
            Cell::Int(i) => Some(*i as f64),
            Cell::Float(f) => Some(*f),
            _ => None,
        }
    }
    fn is_null(&self) -> bool {
        matches!(self, Cell::Null)
    }
}

type Row = HashMap<&'static str, Cell>;

fn fixture() -> Vec<Row> {
    let mk = |id: i64, amount: f64, kind: Option<&str>| -> Row {
        let mut r = HashMap::new();
        r.insert("id", Cell::Int(id));
        r.insert("amount", Cell::Float(amount));
        r.insert(
            "kind",
            match kind {
                Some(k) => Cell::Text(k.to_string()),
                None => Cell::Null,
            },
        );
        r
    };
    vec![
        mk(1, 10.5, Some("a")),
        mk(2, 3.0, Some("b")),
        mk(3, 7.25, None),
        mk(4, 22.0, Some("a")),
        mk(5, 5.0, Some("c")),
    ]
}

// --- reference evaluator -------------------------------------------------

fn lit_cmp(cell: &Cell, lit: &Literal) -> Option<Ordering> {
    match lit {
        Literal::Int(i) => cell.as_f64()?.partial_cmp(&(*i as f64)),
        Literal::Float(f) => cell.as_f64()?.partial_cmp(f),
        Literal::Text(s) => match cell {
            Cell::Text(c) => Some(c.as_str().cmp(s.as_str())),
            _ => None,
        },
        _ => None,
    }
}

fn eval_filter(row: &Row, f: &Filter) -> bool {
    match f {
        Filter::Compare { field, op, value } => {
            let cell = &row[field.as_str()];
            if cell.is_null() {
                return false; // NULL <op> x is never true
            }
            match lit_cmp(cell, value) {
                Some(ord) => match op {
                    CompareOp::Eq => ord == Ordering::Equal,
                    CompareOp::Ne => ord != Ordering::Equal,
                    CompareOp::Lt => ord == Ordering::Less,
                    CompareOp::Le => ord != Ordering::Greater,
                    CompareOp::Gt => ord == Ordering::Greater,
                    CompareOp::Ge => ord != Ordering::Less,
                },
                None => false,
            }
        }
        Filter::In { field, values } => {
            let cell = &row[field.as_str()];
            !cell.is_null()
                && values
                    .iter()
                    .any(|v| lit_cmp(cell, v) == Some(Ordering::Equal))
        }
        Filter::IsNull { field } => row[field.as_str()].is_null(),
        Filter::IsNotNull { field } => !row[field.as_str()].is_null(),
        Filter::Between { field, low, high } => {
            let cell = &row[field.as_str()];
            !cell.is_null()
                && lit_cmp(cell, low)
                    .map(|o| o != Ordering::Less)
                    .unwrap_or(false)
                && lit_cmp(cell, high)
                    .map(|o| o != Ordering::Greater)
                    .unwrap_or(false)
        }
        Filter::And(subs) => subs.iter().all(|s| eval_filter(row, s)),
        Filter::Or(subs) => subs.iter().any(|s| eval_filter(row, s)),
        Filter::Not(inner) => !eval_filter(row, inner),
        Filter::Like { .. } => unimplemented!("LIKE not covered by the reference evaluator"),
    }
}

/// NULLs sort first (SQLite ASC default); otherwise numeric/text ordering.
fn order_cells(a: &Cell, b: &Cell) -> Ordering {
    match (a.is_null(), b.is_null()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => match (a.as_f64(), b.as_f64()) {
            (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
            _ => match (a, b) {
                (Cell::Text(x), Cell::Text(y)) => x.cmp(y),
                _ => Ordering::Equal,
            },
        },
    }
}

fn eval_reference(q: &Query, rows: &[Row]) -> Vec<Vec<JsonValue>> {
    let mut selected: Vec<&Row> = rows
        .iter()
        .filter(|r| q.filters.iter().all(|f| eval_filter(r, f)))
        .collect();

    for ob in q.order_by.iter().rev() {
        let OrderBy { field, direction } = ob;
        selected.sort_by(|a, b| {
            let ord = order_cells(&a[field.as_str()], &b[field.as_str()]);
            match direction {
                Direction::Asc => ord,
                Direction::Desc => ord.reverse(),
            }
        });
    }

    if let Some(limit) = q.limit {
        selected.truncate(limit as usize);
    }

    selected
        .into_iter()
        .map(|r| q.fields.iter().map(|f| r[f.as_str()].to_json()).collect())
        .collect()
}

// --- harness -------------------------------------------------------------

async fn sqlite_with_fixture() -> SqliteDriver {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("CREATE TABLE t (id INTEGER PRIMARY KEY, amount REAL, kind TEXT)")
        .execute(&pool)
        .await
        .unwrap();
    for r in fixture() {
        let amount = r["amount"].as_f64().unwrap();
        let kind = match &r["kind"] {
            Cell::Text(s) => Some(s.clone()),
            _ => None,
        };
        let id = match r["id"] {
            Cell::Int(i) => i,
            _ => unreachable!(),
        };
        sqlx::query("INSERT INTO t (id, amount, kind) VALUES (?, ?, ?)")
            .bind(id)
            .bind(amount)
            .bind(kind)
            .execute(&pool)
            .await
            .unwrap();
    }
    SqliteDriver::from_pool(pool)
}

async fn assert_same(driver: &SqliteDriver, q: &Query) {
    let compiled = gauss_query::compile(q, &gauss_query::SqliteDialect).unwrap();
    let engine = driver.run(&compiled).await.unwrap();
    let reference = eval_reference(q, &fixture());
    assert_eq!(
        engine.rows, reference,
        "engine vs reference mismatch for query: {q:?}\nSQL: {}",
        compiled.sql
    );
}

#[tokio::test]
async fn differential_filter_order_limit() {
    let d = sqlite_with_fixture().await;

    // amount >= 5.0, newest first, top 3
    let mut q1 = Query::new("t");
    q1.fields = vec!["id".into(), "kind".into()];
    q1.filters = vec![Filter::Compare {
        field: "amount".into(),
        op: CompareOp::Ge,
        value: Literal::Float(5.0),
    }];
    q1.order_by = vec![OrderBy {
        field: "id".into(),
        direction: Direction::Desc,
    }];
    q1.limit = Some(3);
    assert_same(&d, &q1).await;

    // kind IN ('a','c') ordered by amount asc
    let mut q2 = Query::new("t");
    q2.fields = vec!["id".into(), "amount".into()];
    q2.filters = vec![Filter::In {
        field: "kind".into(),
        values: vec![Literal::Text("a".into()), Literal::Text("c".into())],
    }];
    q2.order_by = vec![OrderBy {
        field: "amount".into(),
        direction: Direction::Asc,
    }];
    assert_same(&d, &q2).await;

    // kind IS NOT NULL, ordered by kind then id
    let mut q3 = Query::new("t");
    q3.fields = vec!["kind".into(), "id".into()];
    q3.filters = vec![Filter::IsNotNull {
        field: "kind".into(),
    }];
    q3.order_by = vec![
        OrderBy {
            field: "kind".into(),
            direction: Direction::Asc,
        },
        OrderBy {
            field: "id".into(),
            direction: Direction::Asc,
        },
    ];
    assert_same(&d, &q3).await;

    // Nested AND/OR/NOT + BETWEEN
    let mut q4 = Query::new("t");
    q4.fields = vec!["id".into()];
    q4.filters = vec![Filter::Or(vec![
        Filter::Between {
            field: "amount".into(),
            low: Literal::Float(5.0),
            high: Literal::Float(10.5),
        },
        Filter::Not(Box::new(Filter::IsNotNull {
            field: "kind".into(),
        })),
    ])];
    q4.order_by = vec![OrderBy {
        field: "id".into(),
        direction: Direction::Asc,
    }];
    assert_same(&d, &q4).await;
}
