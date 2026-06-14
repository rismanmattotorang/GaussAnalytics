# GaussAnalytics — Delivery Roadmap

> **Owner:** Gaussian Technologies

Phased, strangler-fig migration. Every phase ends with compiling code, tests,
and a runnable artifact. Phases 0 and 1 are delivered in this changeset.

---

## Phase 0 — Foundation ✅ (this changeset)

**Goal:** a credible, buildable skeleton and the full strategy.

- [x] Cargo workspace + crate layout + shared dependency pinning.
- [x] Strategy, architecture, porting map, roadmap, and ADR docs.
- [x] Startup-style README, de-branded (no reference-product naming).
- [x] CI workflow (fmt, clippy, build, test) and dev tooling.
- [x] `.gitignore`, `rust-toolchain.toml`, license retained.
- [x] Frontend reuse directory established (React/TS/JS/CSS scaffold).

**Exit criteria:** `cargo check --workspace` succeeds; docs reviewable.

---

## Phase 1 — Core engine skeleton ✅ (this changeset)

**Goal:** the spine of the platform compiles, runs, and is tested.

- [x] `gauss-core`: domain entities + **GQL** query AST (JSON round-trip).
- [x] `gauss-query`: GQL → **parameterized** SQL compiler with a `Dialect`
      trait (Postgres/MySQL/SQLite/Generic) + unit tests.
- [x] `gauss-config`: layered, typed configuration (defaults → env).
- [x] `gauss-auth`: Argon2 hashing, sessions, roles, permission model + tests.
- [x] `gauss-db`: repository traits + in-memory implementation + tests.
- [x] `gauss-mcp-gateway`: `McpGateway` trait + HTTP client + policy/audit hook.
- [x] `gauss-nl2sql`: `Nl2Sql` trait + HTTP client + guardrail pipeline.
- [x] `gauss-server`: axum app — health, version, dataset (compile), nl2sql,
      mcp, plus static asset serving.
- [x] `gauss-tui`: Ratatui admin console skeleton (tabbed, event loop).
- [x] `gaussctl`: CLI with `serve | admin | migrate | version`.

**Exit criteria:** `cargo test --workspace` green; `gaussctl serve` answers
`/api/health`; `gaussctl admin` launches the TUI.

---

## Phase 2 — Persistence, drivers, real auth ✅ (complete)

Delivered (compiling + tested):

- [x] `gauss-db` on `sqlx` (SQLite) with SQL migrations + `gaussctl migrate`;
      `SqliteStore` implements the same repository traits as the in-memory store.
- [x] `gauss-drivers`: a `Driver` trait + working SQLite driver that executes a
      `CompiledQuery` and returns rows.
- [x] Schema sync + field classification (SQLite type-affinity) via the driver.
- [x] Login / logout / session resolution + a permission gate on dataset
      compilation (authenticated callers are permission-checked).
- [x] Golden-file query tests across Postgres / MySQL / SQLite / Generic.
- [x] Live query **execution** endpoint `POST /api/dataset/run`: compile GQL,
      build a driver from the source's connection, run it, return rows.
- [x] `serve()` runs on the persistent `SqliteStore` (creates the file, runs
      migrations, seeds demo idempotently, bootstraps an admin from env).
- [x] Data-source management endpoints (admin-gated): `POST /api/databases`
      (register), `POST /api/databases/{id}/sync` (introspect + persist tables),
      `GET /api/databases/{id}/tables` (list synced tables).
- [x] **PostgreSQL** store (`PgStore`) and driver (`PgDriver`) behind the same
      traits; `build_store`/`migrate` dispatch by URL scheme. (Live PG tests are
      `#[ignore]`d — set `GAUSS_TEST_PG_URL` to run them.)
- [x] `GET /api/users` (admin) + the **TUI's Databases/Users/Overview tabs read
      live data** from the server (`GAUSS_API_URL` / `GAUSS_API_TOKEN`, `r` to
      refresh).
- [x] **MySQL** store (`MySqlStore`, dedicated migration set) and driver
      (`MySqlDriver`) behind the same traits; dispatch by URL scheme. (Live
      MySQL tests are `#[ignore]`d — set `GAUSS_TEST_MYSQL_URL`.)
- [x] **Mandatory-auth tower middleware** (`require_auth`) gating all API routes
      except a small public set, plus static **service API keys**
      (`GAUSS_API_KEYS`, constant-time compare → service-admin principal).

- [x] **Fingerprinting + semantic typing** during sync: `Driver::fingerprint`
      computes per-column value stats (rows/nulls/distinct); sync infers a
      `SemanticType` (Category/Quantity/Temporal/Text/Key) and stores both.
- [x] **Persisted per-user grants** (`permission_grants` table + endpoints
      `GET/POST/DELETE /api/users/{id}/grants`); `authenticate` builds a
      `PermissionSet` from stored grants. Read-path gating now honors them.
- [x] **DB-backed rotatable API keys** (`api_keys` table, SHA-256 hashed;
      `POST/GET /api/api-keys`, `POST /api/api-keys/{id}/revoke`); accepted via
      `X-API-Key`/`Bearer` and resolved to the owning user's permissions.
- [x] **Differential testing harness**: identical GQL executed by the SQLite
      driver and an independent in-Rust reference evaluator must match (CI-run).
- [x] **Contract-compatibility suite**: a server test exercises every endpoint
      the reused frontend client depends on (status + JSON shape).

**Exit criteria:** `cargo test --workspace` green; all three SQL backends
implemented behind one set of traits; auth enforceable end-to-end.

> Note: live Postgres/MySQL tests are `#[ignore]`d (need running servers); they
> are compile-verified in CI. Provide `GAUSS_TEST_PG_URL` / `GAUSS_TEST_MYSQL_URL`
> to run them.

---

## Phase 3 — Scheduling, alerts, embedding, caching ✅ (complete)

- [x] **Async job scheduler** (`gauss-scheduler`): `Job`/`Notifier` traits + a
      deterministic, unit-tested `Scheduler`; wired into `serve()` to refresh
      synced sources on an interval (`GAUSS_SCHEDULER_PERIOD_SECS`).
- [x] **Query alerts** (`AlertJob`): run a GQL query, notify when results cross
      a threshold; pluggable `Notifier` (log + collecting). Refresh job re-syncs
      connected sources.
- [x] **Signed-token embedding** (`gauss-auth::embed`): HMAC-SHA256 tokens,
      constant-time verified; endpoints `POST /api/embed/token` (admin) and
      `GET /api/embed/resolve` (`GAUSS_EMBEDDING_SECRET`).
- [x] **Query/result caching** (`gauss-server::cache`): compiled-query-keyed TTL
      cache wired into `/api/dataset/run` (`GAUSS_CACHE_TTL_SECS`).

See [`COMPARISON.md`](./COMPARISON.md) for how these stack up against the
reference platform.

---

## Phase 4 — Content, web UX, portability, hardening (in progress)

Delivered (compiling + tested):

- [x] **Saved questions, dashboards, collections** — persisted via a generic
      `ContentRepository` (one table, typed JSON) across all four stores;
      endpoints to create/list/get/delete + **run a saved question**.
- [x] **Content export/import** — `GET /api/export` / `POST /api/import`
      (admin) for portable bundles of collections/cards/dashboards.
- [x] **Web UI** on the reused React frontend: a **query builder** (pick
      source/table/fields, filter, summarize + group-by), **run with table +
      bar chart**, **save questions**, **saved-questions runner**, and a
      **natural-language Ask** panel — all on the typed API client.
- [x] **Performance harness** (`cargo run -p gauss-query --example
      bench_compile`) + **`deny.toml`** supply-chain policy for `cargo deny`.

- [x] **Multi-turn NL2SQL** — requests carry prior turns (`history`) for
      refinement/clarification; threaded through the pipeline + API.
- [x] **MCP agent workflows** — `POST /api/mcp/workflow` chains tool calls,
      each policy-checked + audited, stop-on-error; auth-gated.
- [x] **Richer visualizations** — table + **bar / line / pie** with a chart
      picker; **dashboards** view (build from saved cards + run them in a grid).
- [x] **SBOM + supply chain in CI** — `cargo metadata` SBOM artifact + a
      `cargo-deny` job; frontend **vitest** tests run in CI.
- [x] **TUI parity** — content (cards/dashboards) counts surfaced in the console.

Remaining (tracked in [`UI_ASSESSMENT.md`](./UI_ASSESSMENT.md)):

- [ ] Pivot tables + more chart types (area/scatter/map/funnel/combo).
- [ ] Drag-and-drop dashboard layout editor, dashboard filters/params.
- [ ] In-UI native SQL editor; models/metrics layer.
- [ ] Subscription delivery channels (email/Slack) UI.
- [ ] Driver long-tail (BigQuery/Snowflake/Redshift/ClickHouse).
- [ ] Make `cargo-deny` a required CI gate once the advisory baseline is clean.

---

## Definition of done (every phase)

1. `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` all pass.
2. New behavior has tests.
3. Security review for anything touching auth, SQL generation, or AI/MCP.
4. Docs updated (this file + ADRs as needed).
