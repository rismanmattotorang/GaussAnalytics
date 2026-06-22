# GaussAnalytics vs. the reference analytics platform

> **Owner:** Gaussian Technologies

GaussAnalytics keeps the proven self-service-BI *product shape* (connect a
database → explore → dashboard → share/embed → alert) but rebuilds the engine to
be **faster, safer, more operable, and AI-native**. This page states where it is
already superior and is honest about what is still in progress.

## Where GaussAnalytics is superior today

| Dimension | Reference platform | **GaussAnalytics** | Why it's better |
|---|---|---|---|
| Runtime | JVM (Clojure) — GC pauses, large RSS, slow cold start | **Rust** (Tokio/axum) — no GC, small RSS, fast cold start | Predictable p99 latency; cheaper to run; instant restarts |
| Query safety | SQL assembled as strings; escaping by discipline | **GQL → parameterized SQL by construction** — user literals are *always* bound params | SQL injection is structurally impossible, not merely mitigated |
| AI | Bolt-on assistant, limited governance | **NL2SQL + MCP behind governed integration layers** — schema grounding, read-only guardrails, per-user permission enforcement, full audit | AI is safe-by-default and policy-gated; same parameterized path as human queries |
| API keys | — | **Rotatable, SHA-256-hashed, DB-backed** + static service keys | Service automation without sharing user passwords or long-lived sessions |
| AuthZ | Role/permission model (web only to administer) | **Value-based permission discharge** + persisted per-user/per-database grants, enforced as an explicit step before every query | "Did we check permission?" is a typed, testable step |
| Operations | Web admin only | **Web admin *plus* a fast Ratatui TUI** reading live data | Operators get a keyboard-driven console over the same API |
| Deploy | Fat JVM artifact | **Single static binary** (`gaussctl`) | Trivial containers, fast autoscale |
| Supply chain | Large transitive JVM tree | **Lean, audited Rust deps**, `#![forbid(unsafe_code)]` across crates | Smaller, auditable attack surface |
| Engine testing | — | **Differential harness** (engine vs. independent reference evaluator) + **golden SQL** across 4 dialects | Semantic drift is caught in CI |
| Embedding | Signed-token embedding | **HMAC-SHA256 signed embed tokens**, constant-time verified, stateless | Same capability, minimal and auditable |
| Caching | Query/result caching | **Compiled-query-keyed TTL cache** (pluggable to Redis) | Identical GQL shares cache entries |
| Scheduling | Quartz | **Lean Tokio scheduler** (deterministic, unit-tested) + query alerts + scheduled notebook-card refresh | No heavyweight scheduler dependency |
| Dashboards | SQL-question tiles, filters, text cards | **All of that** (drag-layout, tabs, dashboard links, cross-filter, parameters, Markdown text cards, embedding, emailed subscriptions) **plus tiles backed by computed notebook outputs** | A dashboard tile can be an arbitrary Python/ML result, not just a SQL query |
| Notebooks→dashboards | — (BI tool has no notebook) | **Publish a notebook cell (chart/big-number/table/matplotlib) as a dashboard card**, snapshot-rendered and schedule-refreshed | Pandas/sklearn/PyTorch outputs become first-class dashboard tiles |
| Charting | Built-in chart library | **nivo/D3** (bar/line/area/funnel/scatter/combo/pie/pivot) shared across questions, dashboards, **and** notebook cells | One consistent, modern D3 chart layer everywhere |

## Performance posture

The hot paths — query compilation, parameter binding, row streaming — run with
no JVM warmup and a fraction of the memory footprint. The query compiler is
allocation-light and fully synchronous; I/O is async end-to-end on Tokio. A
single binary cold-starts in milliseconds.

Measured: the bundled harness (`cargo run --release -p gauss-query --example
bench_compile`) compiles a representative analytical query (select + nested
filters + aggregation + group-by + order + limit) to parameterized SQL at
**~500,000 queries/sec (~1.9 µs each) on a single core** — query *planning* is
effectively free relative to network/DB round-trips.

## Security posture

- **Parameterized-by-construction** SQL (see `gauss-query`): no code path emits a
  user literal as SQL text.
- **Argon2id** password hashing; **opaque** server-side sessions (immediate
  revocation); **SHA-256** API keys (only hashes stored).
- **Mandatory-auth middleware** (`require_auth`) + per-user persisted grants
  enforced before query execution.
- **Governed AI**: NL2SQL output is read-only-guardrailed and runs under the
  caller's permissions; MCP tool calls are allow-listed and audited.
- `#![forbid(unsafe_code)]` workspace-wide; lean dependency tree.

## Honest gaps (in progress)

GaussAnalytics does **not yet** match the reference platform's *breadth*:

- **Visualization depth**: the web UI now ships the **nivo/D3** chart library
  (bar/line/area/funnel/scatter/combo/pie/pivot), a **drag-and-drop dashboard
  layout editor** (reorder, per-tile width, tabs, links, cross-filter), and
  **embedded notebooks** whose chart/big-number/table/image cells publish onto
  dashboards. Remaining: geospatial **maps** and deeper drill-through config.
- **Driver breadth**: SQLite/Postgres/MySQL plus **BigQuery/Snowflake/ClickHouse**
  (the latter three as integration-stage REST drivers with golden-tested
  dialects, live-validation pending); further long tail (Redshift, Databricks,
  …) per demand behind the `Driver` trait.
- **AI depth**: NL2SQL + MCP are integrated and governed, but multi-turn
  clarification, lineage, and chained MCP agent workflows are still ahead.
- **Subscriptions/pulses**: the scheduler now delivers query alerts and
  subscription digests over **log, webhook, and email** channels (email via an
  HTTP relay / transactional API). Per-recipient schedule management UI and a
  broader **row-level sandboxing** model are still scheduled.
- **Dashboard text/markdown cards** ✅ and **runtime-editable, persisted AI
  settings** (provider/model/key edited in the UI, pipeline hot-swapped with no
  restart) ✅ are now implemented. Deeper drill-through configuration is ahead.
- A full **SBOM/`cargo deny` CI gate** is configured (`deny.toml`) but not yet
  wired as a required CI job.

The thesis holds: match the product surface area over time, while being
*structurally* better on performance, safety, operability, and governed AI —
and the BI core (sources → explore → save → dashboard → embed → alert →
export) now exists end-to-end.
