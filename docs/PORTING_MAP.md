# GaussAnalytics — Clojure/Python → Rust Porting Map

> **Owner:** Gaussian Technologies

A subsystem-by-subsystem map from the reference platform's Clojure/Python
implementation to the GaussAnalytics Rust stack. Frontend TypeScript/JavaScript
/CSS is **reused** and therefore not listed as a port.

Legend — **Phase**: when it lands (see [`ROADMAP.md`](./ROADMAP.md)).
**Status**: ☐ planned · ◐ scaffolded (Phase 0/1) · ☑ implemented.

| Reference subsystem (Clojure/Python) | Responsibility | Rust target | Key Rust deps | Phase | Status |
|---|---|---|---|---|---|
| HTTP API (Ring/Compojure-style) | Routing, JSON, middleware | `gauss-server` (axum) | `axum`, `tower-http`, `serde_json` | 1 | ◐ |
| MBQL query AST | Structured query representation | `gauss-core::gql` | `serde` | 1 | ◐ |
| Query processor (MBQL→SQL/native) | Compile + execute queries | `gauss-query` | (std) | 1 | ◐ |
| Dialect/driver SQL generation | Per-DB SQL quirks | `gauss-query::dialect` | (std) | 1→2 | ◐ |
| Toucan2 ORM + entities | App metadata persistence | `gauss-db` repositories + `SqliteStore` | `sqlx` | 1→2 | ◐ |
| Liquibase migrations | Schema versioning | SQL migrations + `gaussctl migrate` | `sqlx::migrate` | 2 | ◐ |
| JDBC data-source drivers | Connect to warehouses | `gauss-drivers` (`Driver` trait + SQLite + Postgres) | `sqlx`, native connectors | 2 | ◐ |
| Auth / sessions | Login, sessions, API keys | `gauss-auth` + server auth (sessions, SHA-256 API keys) | `argon2`, `sha2` | 1→2 | ☑ |
| Permissions / RBAC | Collection/DB-scoped access | `gauss-auth::perms` + persisted grants + route gate | (std) | 1→2 | ☑ |
| Database sync / fingerprint / scan | Discover schema + stats | `gauss-drivers::sync_schema` + `fingerprint` | `sqlx`, `tokio` | 2 | ☑ |
| Quartz scheduler | Cron jobs, alerts, refresh | `gauss-scheduler` | `tokio`, cron lib | 3 | ☐ |
| Pulses / alerts / subscriptions | Scheduled delivery | `gauss-notify` | `tokio`, `reqwest`/SMTP | 3 | ☐ |
| Embedding (signed tokens) | Embedded analytics | `gauss-auth::embed` | JWT lib | 3 | ☐ |
| Caching | Query/result caching | `gauss-cache` | `moka`/redis | 3 | ☐ |
| Serialization (export/import) | Portable app content | `gauss-serdes` | `serde` | 4 | ☐ |
| AI glue (MetaBot-style + Python bits) | NL→query, assistance | `gauss-nl2sql` (integration) | `reqwest` → Gaussian NL2SQL | 1→2 | ◐ |
| Agent/tool integration | External tools/actions | `gauss-mcp-gateway` (integration) | `reqwest` → Gaussian MCP | 1→2 | ◐ |
| Admin console (web-only) | Operations | `gauss-tui` (Ratatui) **+** web | `ratatui`, `crossterm` | 1→2 | ◐ |

## Notable porting decisions

- **MBQL → GQL.** We re-specify the query AST as **GQL** with stable JSON
  serialization so the reused frontend can construct queries unchanged (the
  field names and shapes are kept compatible at the API boundary).
- **Multimethod drivers → trait objects.** The reference engine dispatches
  driver behavior via Clojure multimethods. In Rust this becomes a `Dialect`
  trait (SQL generation) plus a `Driver`/connection trait (execution), with one
  implementation per supported database.
- **Toucan2 → repository traits.** We hide persistence behind narrow repository
  traits. Phase 1 uses in-memory implementations; Phase 2 swaps in `sqlx`
  without changing call sites — a clean strangler boundary.
- **Python AI → integration layer.** Any Python in the reference AI path is not
  ported line-for-line; AI is delegated to Gaussian's NL2SQL and MCP services,
  and GaussAnalytics owns only the typed, governed integration.
- **core.async → Tokio.** Async orchestration moves to Tokio tasks/streams.
