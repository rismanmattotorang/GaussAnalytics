# ADR 0002 — GQL: the GaussAnalytics query AST and compiler

- **Status:** Accepted
- **Date:** 2026-06-14
- **Owner:** Gaussian Technologies

## Context

The reference engine represents questions as MBQL — a structured, nested query
AST compiled to SQL/native per database driver. This is the heart of the
product. We must reproduce its essential capability while making safety
structural.

## Decision

Define **GQL** (Gauss Query Language) in `gauss-core::gql`: a serializable AST
(source table, field selection, filters, aggregations, breakouts, ordering,
limit) that round-trips to/from JSON so the reused frontend can build queries.

Compile GQL in `gauss-query` to a `CompiledQuery { sql, params }` where:
- `sql` contains **only placeholders** — no user literal is ever interpolated
  into SQL text;
- `params` is a typed vector bound at execution time;
- dialect differences (identifier quoting, placeholder style, `LIMIT` syntax)
  live behind a `Dialect` trait with one impl per database.

## Consequences

- **+** SQL injection is prevented *by construction*: the compiler cannot emit a
  user literal as SQL text.
- **+** New databases are added by implementing one trait.
- **+** GQL is testable in isolation (fixtures → expected SQL).
- **−** Some highly database-specific features need per-dialect escape hatches;
  we add a native-query path (validated) for those, gated by permissions.
- NL2SQL output is validated/compiled through this same path, so AI-generated
  queries inherit the same safety guarantees.
