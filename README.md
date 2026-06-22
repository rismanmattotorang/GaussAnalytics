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
- 📊 **Rich, interactive charts** — every result and dashboard card renders
  through [**nivo**](https://nivo.rocks) (D3-powered React charts): responsive
  bar, line, area, scatter, funnel, combo, and pie with tooltips, legends, axis
  titles, animation, and click-to-cross-filter.

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
| `GAUSS_NL2SQL_ENABLED` / `GAUSS_NL2SQL_PROVIDER` | `false` / `mock` | in-house NL2SQL engine (LLM provider) |
| `GAUSS_MCP_ENABLED` / `GAUSS_MCP_BASE_URL` | `false` | Gaussian MCP Gateway integration |

See [`.env.example`](.env.example) for the full list.

## How it works

```
   Web UI (React/TS) ─┐
                      ├─► gauss-server (Rust/axum, HTTP+JSON API)
   Admin TUI ─────────┘        │
                               ├─ gauss-query    GQL → parameterized SQL
                               ├─ gauss-db       metadata store
                               ├─ gauss-auth     sessions · RBAC
                               ├─ gauss-nl2sql   in-house NL2SQL (in-process LLM + guardrails)
                               └─ gauss-mcp-gateway ─► Gaussian MCP (integration)

   Conversational chat ─┐
   Chat Web UI (SSE) ───┼─► gauss-chat / gauss-chat-tui  (in-process agent loop)
   Chat TUI (ratatui) ──┘        │
                                 ├─ gauss-engine   agent loop · tools · UI components
                                 ├─ gauss-tools    run_sql · text_to_sql · visualize · memory
                                 ├─ gauss-llm      LLM clients (mock/openai/anthropic/ollama/gemini)
                                 └─ gauss-sql      SQL runners (sqlite/postgres/snowflake)
```

- **GQL** is GaussAnalytics' structured query language: the web UI builds it,
  the server validates it against your schema, and the compiler emits
  parameterized SQL per database dialect.
- **AI is in-house and governed.** The NL2SQL engine runs entirely in-process —
  no external service, no service credential. Translation drives a configured
  LLM provider directly; every generated query passes GaussAnalytics' own
  guardrails (schema grounding, read-only validation, table allowlist, per-user
  permissions, PII redaction, and a full audit trail).
- **Two ways to ask.** Beyond the BI web UI, GaussAnalytics ships a
  conversational **chat web UI** (self-contained, SSE/WebSocket streaming, CSV
  upload) and a keyboard-driven **chat TUI** — both backed by the same
  self-correcting agent loop and tools.

Read the [Architecture](docs/ARCHITECTURE.md) and [Strategy](docs/STRATEGY.md)
for the full picture.

## Data sources

GaussAnalytics connects to a range of engines through one [`Driver`] trait and a
per-engine SQL [`Dialect`]. The GQL compiler renders the **correct SQL for each**
— identifier quoting, parameter placeholders, and row-limit syntax all vary by
dialect — so the same question runs safely everywhere:

| Engine | Driver | Identifiers | Placeholders | Row limit |
|---|---|---|---|---|
| SQLite | in-process (`sqlx`) | `"col"` | `?` | `LIMIT n` |
| PostgreSQL | `sqlx` pool | `"col"` | `$n` | `LIMIT n` |
| MySQL | `sqlx` pool | `` `col` `` | `?` | `LIMIT n` |
| Oracle | ORDS REST | `"col"` | `:n` | `OFFSET 0 ROWS FETCH NEXT n ROWS ONLY` |
| Snowflake | SQL REST API | `"col"` | `?` | `LIMIT n` |
| BigQuery · ClickHouse | REST/HTTP | dialect-specific | dialect-specific | `LIMIT n` |

A kind's name is a single canonical string (`"sqlite"`, `"oracle"`, …) shared by
the API, the metadata store, and the frontend, so they can never disagree. Every
value a user supplies is a **bound parameter** — never SQL text — on all engines.

## Project layout

```
crates/
  gauss-core          domain model + GQL query AST
  gauss-query         GQL → parameterized SQL compiler
  gauss-config        layered configuration
  gauss-auth          Argon2 hashing · sessions · RBAC
  gauss-db            metadata store (repository traits · in-memory · sqlx SQLite/Postgres)
  gauss-drivers       data-source drivers (SQLite · Postgres · MySQL · Oracle · Snowflake · BigQuery · ClickHouse): execute · discover · fingerprint
  gauss-scheduler     background job engine (schema refresh, query alerts)
  gauss-mcp-gateway   integration layer → Gaussian MCP Servers
  gauss-nl2sql        in-house NL2SQL: grounding + in-process LLM translation + guardrails
  gauss-server        axum HTTP/JSON API + static web UI hosting
  gauss-tui           Ratatui operator administration console
  gaussctl            CLI: serve | admin | migrate | version
  -- in-house NL2SQL engine (self-correcting text-to-SQL) --
  gauss-engine        agent loop · tool registry · models · UI components
  gauss-semantic      semantic / modeling-definition layer
  gauss-sqlguard      SQL AST guardrails (read-only · allowlist · LIMIT)
  gauss-llm           LLM clients (mock/openai/anthropic/ollama/gemini/vllm)
  gauss-sql           SQL runners (sqlite · postgres · snowflake) + CSV ingest
  gauss-textsql       self-correcting text-to-SQL pipeline
  gauss-chart         chart generation: deterministic Vega-Lite recommender + Plotly figures
  gauss-insight       GenBI result intelligence: chart + summary + grounded follow-ups
  gauss-tools         built-in agent tools (run_sql · visualize · files · memory)
  gauss-embed         text-embedding providers (hashing/ollama/openai)
  gauss-memory        vector-backed agent memory
  gauss-runtime       shared runtime assembly (LLM selection · demo DB seeding)
  -- conversational UIs --
  gauss-chat          conversational web UI: axum SSE/WebSocket chat server + agent runner binary
  gauss-chat-tui      conversational terminal UI (ratatui) — in-process chat client
frontend/             React + TypeScript web application (charts via nivo/D3)
docs/                 strategy, architecture, roadmap, ADRs
```

## Conversational chat (Web UI + TUI)

Alongside the BI web app, GaussAnalytics includes a chat experience driven by an
in-process agent loop — the same engine, tools, and guardrails, exposed two ways:

```bash
# Conversational web UI (self-contained HTML, SSE streaming, CSV upload).
# Defaults to the offline mock LLM and a seeded demo database.
cargo run -p gauss-chat -- --port 8000
#   → open http://127.0.0.1:8000

# Pick a provider (reads the provider's own API key from the environment):
cargo run -p gauss-chat -- --llm openai --model gpt-4o-mini
# Ground the self-correcting text_to_sql tool in a semantic model:
cargo run -p gauss-chat -- --semantic-model model.yaml

# Conversational terminal UI:
cargo run -p gauss-chat-tui -- --llm mock
#   /load <file.csv> [table]  imports a CSV you can then query in plain English
```

Both clients stream rich components (text, tables, charts, status, task tracker),
support multi-turn conversations and CSV upload, and run every query through the
read-only guardrails. Configuration is via flags or `GAUSS_CHAT_*` environment
variables (`GAUSS_CHAT_LLM`, `GAUSS_CHAT_MODEL`, `GAUSS_CHAT_DB`,
`GAUSS_CHAT_PORT`, `GAUSS_CHAT_SEMANTIC_MODEL`, …).

### Result intelligence (GenBI) — better than WrenAI

Every query result is automatically accompanied by a **GenBI panel**: a
recommended chart, a plain-language summary, and grounded follow-up questions.
Inspired by WrenAI's GenBI, but implemented to be strictly better — the whole
panel is computed in-process from the returned rows, with **no extra LLM
round-trip**:

| | WrenAI | GaussAnalytics (`gauss-insight` + `gauss-chart`) |
|---|---|---|
| Chart selection | extra LLM call → Vega-Lite | **deterministic** from data shape (column kinds + cardinality) |
| Cost / latency | tokens + a round-trip per chart | **free, instant, reproducible** |
| Hallucination | LLM can reference columns the query never returned | **structurally impossible** — only real columns are used |
| Result summary | LLM (can misstate figures) | **computed from the rows** — never misstates a number |
| Follow-ups | LLM-authored | **grounded** in the actual schema/columns |
| Rendering | needs the Vega CDN | **inline, no CDN** (air-gap friendly) — Vega-Lite still emitted for interop |

Chart types: number, bar, grouped bar, line, multi-line, pie, scatter (falling
back to a table when no chart is honest). The panel appears in the chat web UI
(inline SVG charts + clickable follow-up chips), the TUI, and the JSON API.

### Charts & dashboards (nivo)

The React web app renders every query result and dashboard card through
[**nivo**](https://nivo.rocks), a D3-powered React charting library. A result
opens with a sensible default chart and a one-click **chart-type picker**:

- **bar** / **funnel** (sorted horizontal bars) / **pie** — categorical
  breakdowns, with **click-to-cross-filter** that drives dashboard parameters;
- **line** / **area** — trends;
- **scatter** — two-measure correlation;
- **combo** — bars + an overlaid line on a shared scale (one custom nivo layer);
- **pivot** / **table** — for matrix and raw views.

All charts share one dark theme and get tooltips, legends, axis titles, and
animation for free, are fully responsive, and are code-split into a separate
cacheable bundle so the app shell stays small. Chart wrappers live in
`frontend/src/components/charts/NivoCharts.tsx` and consume the pure data
helpers in `frontend/src/lib/viz.ts`, keeping the rest of the app decoupled
from the charting library.

## Status

GaussAnalytics is in active development. **Phases 0–3 are complete and Phase 4
is well underway.** Highlights: persistent storage (`sqlx`, **SQLite / Postgres
/ MySQL**), drivers that **execute queries, discover schema, and fingerprint
columns**, auth (sessions, **persisted per-user grants**, mandatory-auth
middleware, **rotatable API keys**, **signed-token embedding**), data-source
management, a **scheduler with query alerts**, a **query-result cache**, and now
the BI core — **saved questions, dashboards, collections, content
export/import**, and a **web UI** (query builder + **interactive nivo/D3 charts**
+ saved questions + natural-language Ask). The **NL2SQL engine is now fully
in-house** —
a self-correcting text-to-SQL pipeline (schema linking → few-shot → AST
guardrails → execution-guided repair → PII redaction) with in-process LLM
clients and no external service credential — surfaced through a **conversational
chat web UI** (SSE/WebSocket streaming + CSV upload) and a **chat TUI**. Plus a
**differential-testing harness**, a **contract-compatibility suite**, an admin
**TUI**, a **compile benchmark** (~500k queries/sec/core), and a **`cargo deny`**
supply-chain policy. `cargo test --workspace` is green.

See [**how GaussAnalytics compares**](docs/COMPARISON.md) to the reference
platform (with an honest list of remaining gaps), and the
[Roadmap](docs/ROADMAP.md).

## Development

```bash
cargo test --workspace        # run all tests
cargo clippy --workspace      # lint
cargo fmt --all               # format
```

## License

[MIT](LICENSE) © 2026 Gaussian Technologies.
