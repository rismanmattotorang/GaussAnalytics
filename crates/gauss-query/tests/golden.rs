//! Golden-file tests: fixed GQL inputs compiled to expected SQL per dialect.
//!
//! These lock down the compiler's output so semantic drift is caught early — the
//! Rust analogue of the reference engine's query-processor regression suite. As
//! more databases are supported, add their dialect rows here.

use gauss_core::gql::{
    AggFunc, Aggregation, CompareOp, Direction, Filter, Literal, OrderBy, Query,
};
use gauss_query::{compile, Dialect, GenericDialect, MySqlDialect, PostgresDialect, SqliteDialect};

/// Build a representative analytical query exercising select, filter,
/// aggregation, breakout, order, and limit.
fn representative_query() -> Query {
    Query {
        source_table: "orders".into(),
        fields: vec![],
        filters: vec![Filter::And(vec![
            Filter::Compare {
                field: "total".into(),
                op: CompareOp::Ge,
                value: Literal::Float(100.0),
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
            field: "revenue".into(),
            direction: Direction::Desc,
        }],
        limit: Some(25),
    }
}

fn assert_sql(dialect: &dyn Dialect, expected: &str, expected_params: usize) {
    let compiled = compile(&representative_query(), dialect).unwrap();
    assert_eq!(compiled.sql, expected, "dialect = {}", dialect.name());
    assert_eq!(compiled.params.len(), expected_params);
}

#[test]
fn postgres_golden() {
    assert_sql(
        &PostgresDialect,
        r#"SELECT "status", SUM("total") AS "revenue" FROM "orders" WHERE ("total" >= $1 AND "status" IN ($2, $3)) GROUP BY "status" ORDER BY "revenue" DESC LIMIT 25"#,
        3,
    );
}

#[test]
fn sqlite_golden() {
    assert_sql(
        &SqliteDialect,
        r#"SELECT "status", SUM("total") AS "revenue" FROM "orders" WHERE ("total" >= ? AND "status" IN (?, ?)) GROUP BY "status" ORDER BY "revenue" DESC LIMIT 25"#,
        3,
    );
}

#[test]
fn generic_golden() {
    assert_sql(
        &GenericDialect,
        r#"SELECT "status", SUM("total") AS "revenue" FROM "orders" WHERE ("total" >= ? AND "status" IN (?, ?)) GROUP BY "status" ORDER BY "revenue" DESC LIMIT 25"#,
        3,
    );
}

#[test]
fn mysql_golden() {
    assert_sql(
        &MySqlDialect,
        "SELECT `status`, SUM(`total`) AS `revenue` FROM `orders` WHERE (`total` >= ? AND `status` IN (?, ?)) GROUP BY `status` ORDER BY `revenue` DESC LIMIT 25",
        3,
    );
}
