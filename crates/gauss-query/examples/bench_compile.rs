//! A tiny throughput harness for the GQL → SQL compiler.
//!
//! Run with: `cargo run --release -p gauss-query --example bench_compile`
//!
//! The compiler is allocation-light and fully synchronous; this measures how
//! many representative analytical queries it compiles per second on one core.

use std::time::Instant;

use gauss_core::gql::{AggFunc, Aggregation, CompareOp, Filter, Literal, OrderBy, Query};
use gauss_query::{compile, PostgresDialect};

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
            direction: gauss_core::gql::Direction::Desc,
        }],
        limit: Some(50),
    }
}

fn main() {
    let q = representative_query();
    let dialect = PostgresDialect;

    // Warm up.
    for _ in 0..1_000 {
        let _ = compile(&q, &dialect).unwrap();
    }

    let n = 500_000u32;
    let start = Instant::now();
    let mut sink = 0usize;
    for _ in 0..n {
        let compiled = compile(&q, &dialect).unwrap();
        sink = sink.wrapping_add(compiled.sql.len());
    }
    let elapsed = start.elapsed();
    let per_sec = n as f64 / elapsed.as_secs_f64();
    println!(
        "compiled {n} queries in {elapsed:?} = {per_sec:.0} queries/sec ({:.2} µs each) [checksum {sink}]",
        elapsed.as_secs_f64() * 1e6 / n as f64,
    );
}
