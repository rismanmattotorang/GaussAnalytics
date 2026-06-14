<div align="center">

# GaussAnalytics

**The fast, secure, AI-native business intelligence platform.**

_by [Gaussian Technologies](https://gaussian.tech)_

Connect a database, ask a question in plain English or point-and-click, and
ship a dashboard — on an engine built in Rust for speed you can feel and a
security surface you can actually audit.

[Quickstart](#quickstart) · [Why GaussAnalytics](#why-gaussanalytics) ·
[Architecture](docs/ARCHITECTURE.md) · [Roadmap](docs/ROADMAP.md) ·
[Strategy](docs/STRATEGY.md)

</div>

---

## What it is

GaussAnalytics lets anyone on your team explore data and build dashboards
without writing SQL — and lets analysts go deep when they need to. It is a
modern take on the self-service BI category, rebuilt from the ground up by
Gaussian Technologies on a **Rust** core:

- ⚡ **Fast by design** — a native, async query engine with no garbage-collector
  pauses, fast cold starts, and a fraction of the memory footprint of
  JVM-based tools.
- 🔒 **Secure by construction** — queries are *generated*, never concatenated:
  every user value is a bound parameter, so SQL injection isn't mitigated, it's
  structurally impossible. Memory-safe Rust removes whole classes of bugs.
- 🤖 **AI-native** — ask questions in natural language (**NL2SQL**) and let
  governed agents take action through the **MCP Gateway** — powered by Gaussian
  Technologies' own models and tooling.
- 🖥️ **Operator-first** — a polished web UI for everyone, plus a fast,
  keyboard-driven **terminal admin console** for the people who run it.

> GaussAnalytics keeps a best-in-class web experience (React/TypeScript) and
> pairs it with a brand-new, high-performance Rust backend.

## Why GaussAnalytics

| | Legacy BI | **GaussAnalytics** |
|---|---|---|
| Backend | JVM (GC pauses, heavy RAM) | **Rust** — predictable latency, small footprint |
| Query safety | string-built SQL, ad-hoc escaping | **parameterized by construction** |
| AI | bolt-on, ungoverned | **NL2SQL + MCP**, grounded, permission-checked, audited |
| Admin | web only | web **+ fast terminal (TUI) console** |
| Deploy | fat artifact | **single static binary** |

## Quickstart

> **Prerequisites:** [Rust](https://rustup.rs) 1.90+ and (for the frontend)
> Node 20+ with `pnpm`.

```bash
# Build the platform
cargo build --release

# Run the server (serves the API and, when built, the web UI)
./target/release/gaussctl serve
#   GaussAnalytics listening on http://127.0.0.1:3000

# In another terminal: launch the operator admin console (TUI)
./target/release/gaussctl admin

# See all commands
./target/release/gaussctl --help     # serve | admin | migrate | version
```

Try the API:

```bash
curl localhost:3000/api/health
# {"status":"ok","name":"GaussAnalytics","version":"0.1.0"}

# Compile a structured (GQL) query to safe, parameterized SQL:
curl -s localhost:3000/api/databases       # grab the demo database id, then:
curl -s -X POST localhost:3000/api/dataset/compile \
  -H 'content-type: application/json' \
  -d '{"database_id":"<id>","query":{"source_table":"orders",
       "aggregations":[{"func":"sum","field":"total","alias":"revenue"}],
       "breakouts":["status"]}}'
# => {"sql":"SELECT \"status\", SUM(\"total\") AS \"revenue\" FROM \"orders\" GROUP BY \"status\"","params":[]}
```

### Configuration

All configuration is environment-driven (`GAUSS_*`); sensible defaults apply.

| Variable | Default | Purpose |
|---|---|---|
| `GAUSS_HOST` / `GAUSS_PORT` | `127.0.0.1` / `3000` | server bind address |
| `GAUSS_STATIC_DIR` | `frontend/dist` | built web UI to serve |
| `GAUSS_DATABASE_URL` | `sqlite://data/gauss.db` | app metadata store (Phase 2) |
| `GAUSS_NL2SQL_ENABLED` / `GAUSS_NL2SQL_BASE_URL` | `false` | Gaussian NL2SQL integration |
| `GAUSS_MCP_ENABLED` / `GAUSS_MCP_BASE_URL` | `false` | Gaussian MCP Gateway integration |

See [`.env.example`](.env.example) for the full list.

## How it works

```
   Web UI (React/TS) ─┐
                      ├─► gauss-server (Rust/axum, HTTP+JSON API)
   Admin TUI ─────────┘        │
                               ├─ gauss-query   GQL → parameterized SQL
                               ├─ gauss-db      metadata store
                               ├─ gauss-auth    sessions · RBAC
                               ├─ gauss-nl2sql  ─► Gaussian NL2SQL  (integration)
                               └─ gauss-mcp-gateway ─► Gaussian MCP (integration)
```

- **GQL** is GaussAnalytics' structured query language: the web UI builds it,
  the server validates it against your schema, and the compiler emits
  parameterized SQL per database dialect.
- **AI is governed.** NL2SQL output and MCP tool calls run through
  GaussAnalytics' own guardrails — schema grounding, read-only validation,
  per-user permissions, and a full audit trail — on top of Gaussian's models.

Read the [Architecture](docs/ARCHITECTURE.md) and [Strategy](docs/STRATEGY.md)
for the full picture.

## Project layout

```
crates/
  gauss-core          domain model + GQL query AST
  gauss-query         GQL → parameterized SQL compiler
  gauss-config        layered configuration
  gauss-auth          Argon2 hashing · sessions · RBAC
  gauss-db            metadata store (repository traits · in-memory · sqlx SQLite/Postgres)
  gauss-drivers       data-source drivers (SQLite, Postgres): execute · discover schema
  gauss-mcp-gateway   integration layer → Gaussian MCP Servers
  gauss-nl2sql        integration layer → Gaussian NL2SQL
  gauss-server        axum HTTP/JSON API + static web UI hosting
  gauss-tui           Ratatui operator administration console
  gaussctl            CLI: serve | admin | migrate | version
frontend/             React + TypeScript web application
docs/                 strategy, architecture, roadmap, ADRs
```

## Status

GaussAnalytics is in active development. **Phases 0 and 1** are complete and
**Phase 2 is well underway**: persistent storage (`sqlx`, SQLite **and
Postgres**, with migrations), data-source drivers that **execute queries and
discover schema**, authentication (login/sessions + permission gating),
admin-gated **data-source management endpoints**, and an admin **TUI that reads
live data** from the server. `cargo test --workspace` is green;
`gaussctl serve`/`migrate`/`admin` all work. See the [Roadmap](docs/ROADMAP.md)
for what's next (MySQL, scheduling, embedding, and deeper AI).

## Development

```bash
cargo test --workspace        # run all tests
cargo clippy --workspace      # lint
cargo fmt --all               # format
```

## License

[MIT](LICENSE) © 2026 Gaussian Technologies.
