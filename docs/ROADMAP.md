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

## Phase 2 — Persistence, drivers, real auth

- [ ] `gauss-db` on `sqlx` (Postgres + SQLite) with SQL migrations + `migrate`.
- [ ] Data-source drivers: Postgres, MySQL, SQLite execution paths.
- [ ] Schema sync, fingerprinting, and field classification.
- [ ] Session middleware, API keys, real permission enforcement on every route.
- [ ] Golden-file query tests across dialects; differential testing harness.
- [ ] Wire the reused frontend's API client against the Rust server (contract
      compatibility suite).

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
