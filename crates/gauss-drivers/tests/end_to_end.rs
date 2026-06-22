//! End-to-end BI journey against a live (in-memory) SQLite source: connect →
//! seed → discover schema → compile a GQL query → execute → profile columns.
//! This exercises the real driver and the real GQL→SQL compiler together, with
//! no external infrastructure, mirroring the "connect a data source and query
//! it" user scenario.

use gauss_core::domain::{DataSourceKind, FieldType};
use gauss_core::gql::{
    AggFunc, Aggregation, CompareOp, Direction, Filter, Literal, OrderBy, Query,
};
use gauss_drivers::{Driver, SqliteDriver};
use gauss_query::{compile, dialect};

async fn seeded() -> SqliteDriver {
    let driver = SqliteDriver::connect("sqlite::memory:").await.unwrap();
    sqlx::query(
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, status TEXT, total REAL, country TEXT)",
    )
    .execute(driver.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO orders (status, total, country) VALUES \
         ('paid', 1200.0, 'US'), ('paid', 800.0, 'US'), ('pending', 500.0, 'DE'), \
         ('paid', 9000.0, 'UK'), ('refunded', 3000.0, 'US')",
    )
    .execute(driver.pool())
    .await
    .unwrap();
    driver
}

#[tokio::test]
async fn connect_discover_query_and_profile() {
    let driver = seeded().await;

    // 1. Discover the schema, as a sync would.
    let tables = driver.sync_schema().await.unwrap();
    let orders = tables
        .iter()
        .find(|t| t.name == "orders")
        .expect("orders table");
    assert_eq!(orders.columns.len(), 4);
    let status = orders.columns.iter().find(|c| c.name == "status").unwrap();
    assert_eq!(status.field_type, FieldType::Text);
    let total = orders.columns.iter().find(|c| c.name == "total").unwrap();
    assert_eq!(total.field_type, FieldType::Float);

    // 2. Build a GQL query — total revenue by status, top-first — and compile it
    //    to parameterized SQL for the source's dialect.
    let query = Query {
        source_table: "orders".into(),
        fields: vec![],
        filters: vec![Filter::Compare {
            field: "total".into(),
            op: CompareOp::Gt,
            value: Literal::Int(100),
        }],
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
        limit: Some(10),
    };
    let dialect = dialect::for_kind(DataSourceKind::Sqlite);
    let compiled = compile(&query, dialect.as_ref()).unwrap();
    // The user literal became a bound parameter, never SQL text.
    assert!(compiled.sql.contains('?'));
    assert!(!compiled.sql.contains("100 "));

    // 3. Execute it against the live source.
    let result = driver.run(&compiled).await.unwrap();
    assert_eq!(result.columns, vec!["status", "revenue"]);
    // paid = 1200+800+9000 = 11000 (the top group); pending excluded? no, >100.
    let top = &result.rows[0];
    assert_eq!(top[0], serde_json::json!("paid"));
    assert_eq!(top[1].as_f64().unwrap(), 11000.0);

    // 4. Profile a column (the fingerprint used for semantic typing).
    let fps = driver
        .fingerprint("orders", &["status".to_string()])
        .await
        .unwrap();
    let (_, fp) = &fps[0];
    assert_eq!(fp.total_rows, 5);
    assert_eq!(fp.null_count, 0);
    assert_eq!(fp.distinct_count, 3); // paid, pending, refunded
}

#[tokio::test]
async fn parameterized_filter_blocks_injection_end_to_end() {
    let driver = seeded().await;
    // A classic injection attempt arrives as a user literal; it must be bound,
    // execute harmlessly (matching nothing), and leave the table intact.
    let mut query = Query::new("orders");
    query.filters = vec![Filter::Compare {
        field: "status".into(),
        op: CompareOp::Eq,
        value: Literal::Text("paid'; DROP TABLE orders; --".into()),
    }];
    let dialect = dialect::for_kind(DataSourceKind::Sqlite);
    let compiled = compile(&query, dialect.as_ref()).unwrap();
    assert!(!compiled.sql.contains("DROP TABLE"));
    let result = driver.run(&compiled).await.unwrap();
    assert_eq!(
        result.rows.len(),
        0,
        "no status equals the injection string"
    );

    // The table still exists and still has its rows.
    let count = driver
        .run(&compile(&Query::new("orders"), dialect.as_ref()).unwrap())
        .await
        .unwrap();
    assert_eq!(count.rows.len(), 5);
}
