# GaussAnalytics — Target Architecture

> **Owner:** Gaussian Technologies

This document describes the target Rust architecture for GaussAnalytics and how
each piece maps onto a Cargo workspace of crates and a small set of deployable
binaries.

---

## 1. Workspace layout

```
gaussanalytics/
├── Cargo.toml                  # workspace manifest
├── crates/
│   ├── gauss-core/             # domain model + GQL (query AST) + error types
│   ├── gauss-config/           # layered configuration (file + env)
│   ├── gauss-query/            # GQL → parameterized SQL compiler (per-dialect)
│   ├── gauss-db/               # app metadata store: repository traits + impls
│   ├── gauss-auth/             # password hashing, sessions, RBAC, permissions
│   ├── gauss-mcp-gateway/      # integration layer → Gaussian MCP Servers
│   ├── gauss-nl2sql/           # integration layer → Gaussian NL2SQL
│   ├── gauss-server/           # axum HTTP/JSON API + static frontend serving
│   ├── gauss-tui/              # Ratatui operator administration TUI
│   └── gaussctl/               # CLI binary: serve | admin | migrate | version
├── frontend/                   # reused React/TS/JS/CSS application
└── docs/                       # strategy, architecture, roadmap, ADRs
```

### Dependency direction (no cycles)

```
gaussctl ─► gauss-server ─► gauss-query ─► gauss-core
   │            │      └──► gauss-db    ─► gauss-core
   │            │      └──► gauss-auth  ─► gauss-core
   │            │      └──► gauss-mcp-gateway ─► gauss-core
   │            │      └──► gauss-nl2sql ─► gauss-query, gauss-core
   └──► gauss-tui ─► gauss-config (+ talks to gauss-server over HTTP)
```

`gauss-core` is the leaf everything depends on; it has no dependency on the web,
DB, or AI layers. This keeps the domain model and query AST reusable by the
server, the TUI, and tests alike.

---

## 2. Crate responsibilities

### `gauss-core`
The shared domain. Defines:
- **Entities:** `User`, `Database`, `Table`, `Field`, `Card` (saved question),
  `Dashboard`, `Collection`, `DataSourceKind`, `FieldType`.
- **GQL** — *Gauss Query Language*, a serializable, structured query AST
  (the spiritual successor to the reference engine's MBQL): source table,
  field selection, filters, aggregations, breakouts (group-by), ordering,
  limit. Designed to round-trip to/from JSON so the frontend can build it.
- **Errors:** a single `CoreError` enum used across crates.

### `gauss-config`
Layered configuration: built-in defaults → config file → environment variables
(prefixed `GAUSS_`). Strongly-typed `AppConfig` with sections for server, app
database, security, MCP gateway, and NL2SQL.

### `gauss-query`
The query compiler. Takes a validated `gauss_core::gql::Query` plus a target
`Dialect` and produces a `CompiledQuery { sql, params }` where `sql` contains
only placeholders and every literal is bound. This is the central **security
guarantee**: user input never becomes SQL text. Dialect-specific quoting,
placeholder style, and limit syntax are isolated behind a `Dialect` trait.

### `gauss-db`
The application's own metadata store (users, databases, cards, dashboards,
permissions, sessions, audit). Exposes **repository traits**; Phase 1 ships an
in-memory implementation for fast iteration and tests. The Postgres/SQLite
`sqlx` implementation and SQL migrations land in Phase 2 behind the same traits.

### `gauss-auth`
Security primitives: Argon2 password hashing/verification, opaque session
tokens, role model (`Admin`/`Editor`/`Viewer`), and a `Permission` /
`PermissionSet` model with collection- and database-scoped grants. Provides the
guard types the server uses to gate query execution.

### `gauss-mcp-gateway`
Thin, typed **integration layer** to Gaussian's MCP Servers. Defines an
`McpGateway` trait (list servers, list tools, invoke a tool) and an
HTTP-backed client. Adds GaussAnalytics-owned concerns on top: a server
registry, per-tool policy/allow-listing, and audit hooks. **No model or MCP
server is reimplemented here.**

### `gauss-nl2sql`
Thin, typed **integration layer** to Gaussian's NL2SQL service. Turns a natural
language prompt + grounded schema context into a candidate query, then runs it
through GaussAnalytics' own guardrails: validate/parse the SQL (or GQL),
enforce the requesting user's permissions, and audit. **The model lives in
Gaussian's platform; we own the governance.**

### `gauss-server`
The axum application. Routes for health, auth/session, databases, cards,
dataset execution (run a GQL query), NL2SQL, and the MCP gateway. Serves the
built frontend as static assets. Cross-cutting middleware: tracing, CORS, and
(Phase 2) auth extraction + rate limiting.

### `gauss-tui`
The operator administration console built with **Ratatui**. Tabbed views:
Overview/health, Databases, Users & sessions, Jobs/scheduler, MCP & AI, and
Logs. Talks to `gauss-server` over its HTTP API (same contract as the web UI),
so the TUI is a first-class API client, not a backdoor.

### `gaussctl`
The single CLI entry point and the binary operators install:
- `gaussctl serve` — run the HTTP server.
- `gaussctl admin` — launch the TUI.
- `gaussctl migrate` — run app-DB migrations (Phase 2).
- `gaussctl version` — build/version info.

---

## 3. Request lifecycles

### Running a saved question (GQL)
```
frontend ──POST /api/dataset {query: GQL}──► gauss-server
  → gauss-auth: resolve session, load PermissionSet
  → gauss-core: validate GQL against synced metadata
  → gauss-auth: assert permission to read the target table
  → gauss-query: compile GQL + Dialect → (sql, params)
  → gauss-db/driver: execute parameterized statement, stream rows
  → gauss-server: shape rows + metadata → JSON ──► frontend renders viz
```

### Natural-language question (NL2SQL)
```
frontend ──POST /api/nl2sql {prompt, database_id}──► gauss-server
  → gauss-nl2sql: assemble grounded schema context (tables/fields/permissions)
  → Gaussian NL2SQL service: prompt + context → candidate SQL/GQL
  → gauss-query / validator: parse + validate candidate (reject unsafe)
  → gauss-auth: enforce requesting user's permissions on referenced tables
  → execute (as above) · audit the prompt, candidate, and outcome
```

### MCP tool invocation
```
agent/UI ──POST /api/mcp/invoke {server, tool, args}──► gauss-server
  → gauss-mcp-gateway: policy check (server + tool allow-list, scopes)
  → Gaussian MCP Server: invoke tool
  → audit request/response · return typed result
```

---

## 4. Deployment shape

- **One binary** (`gaussctl`) ships the server, the TUI, and admin commands.
- Stateless server processes scale horizontally; state lives in the app
  database (Phase 2) and the connected data sources.
- Static frontend assets are embedded or served from disk by `gauss-server`.
- Integration with Gaussian MCP/NL2SQL is over the network via configured
  endpoints + credentials; no AI weights are bundled.

---

## 5. Cross-cutting concerns

- **Observability:** `tracing` spans across request handlers, query compilation,
  and integration calls; structured JSON logs in production.
- **Security:** parameterized SQL by construction; Argon2 password hashing;
  permission discharge before execution; auditable AI/MCP calls; lean,
  audited dependency tree.
- **Performance:** async end-to-end (Tokio); streaming result sets; no GC;
  compile-time-resolved dispatch on hot paths.
- **Configuration:** one typed `AppConfig`, sourced from defaults → file → env.
