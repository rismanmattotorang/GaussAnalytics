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

## Phase 2 — Persistence, drivers, real auth (in progress)

Scaffolded in this changeset (compiling + tested):

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

Remaining for Phase 2:

- [ ] Postgres + MySQL store and driver implementations (same `sqlx` pattern).
- [ ] Fingerprinting (value stats) and richer semantic typing during sync.
- [ ] API keys; make authentication mandatory on read paths once
      per-database/-collection grants are persisted; session middleware as a
      tower layer.
- [ ] Differential testing harness (compare results across engines).
- [ ] Contract-compatibility suite exercising the reused frontend client.

---

## Phase 3 — Scheduling, alerts, embedding, caching

- [ ] Async job scheduler (refresh, sync, alerts).
- [ ] Pulses/alerts/subscriptions with delivery channels.
- [ ] Signed-token embedding for embedded analytics.
- [ ] Query/result caching layer.

---

## Phase 4 — AI depth, content portability, polish

- [ ] Deepen NL2SQL governance: clarifying questions, multi-turn, lineage.
- [ ] MCP agent workflows (tool chaining under policy + audit).
- [ ] Content export/import (portable dashboards/questions).
- [ ] Web UI/UX refresh on the reused frontend; TUI feature parity for ops.
- [ ] Performance hardening + SBOM/supply-chain automation per release.

---

## Definition of done (every phase)

1. `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` all pass.
2. New behavior has tests.
3. Security review for anything touching auth, SQL generation, or AI/MCP.
4. Docs updated (this file + ADRs as needed).
