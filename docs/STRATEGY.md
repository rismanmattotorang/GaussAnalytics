# GaussAnalytics — Porting & Modernization Strategy

> **Owner:** Gaussian Technologies
> **Status:** Living document — Phase 0/1 in progress
> **Audience:** Engineering, Product, Security

GaussAnalytics is Gaussian Technologies' AI-native business intelligence
platform. Its product surface (self-service exploration, dashboards, SQL,
embedded analytics, alerts) is inspired by the proven open-source analytics
category leader, but its **engine is being rebuilt on a Rust stack** for
better performance, memory safety, and a smaller, auditable security surface.

This document explains *why* we are porting, *what* we are porting, and the
*principles* that govern the migration. The concrete sequencing lives in
[`ROADMAP.md`](./ROADMAP.md); the architecture lives in
[`ARCHITECTURE.md`](./ARCHITECTURE.md); the subsystem-by-subsystem migration
table lives in [`PORTING_MAP.md`](./PORTING_MAP.md).

---

## 1. The reference system

The reference analytics platform is a mature product with a well-understood
architecture:

| Layer | Reference stack | Notes |
|-------|-----------------|-------|
| Web frontend | React + Redux + TypeScript/JavaScript, Emotion/CSS | Large, battle-tested UX. **We reuse this.** |
| HTTP API | Clojure (Ring/Compojure-style routing) | Rebuild in Rust. |
| Query engine (MBQL) | Clojure — nested query AST compiled to SQL/native | Rebuild as **GQL** in Rust. |
| ORM / app database | Toucan2 over JDBC, Liquibase migrations | Rebuild on `sqlx` + SQL migrations. |
| Data-source drivers | JDBC-based driver multimethods | Rebuild on Rust async drivers. |
| Background jobs | Quartz scheduler | Rebuild on a Rust async scheduler. |
| Auxiliary tooling/AI glue | Clojure + some Python | Rebuild in Rust; AI delegated to Gaussian services. |

### What we keep vs. what we replace

- **Keep (reuse as-is, rebranded):** TypeScript, JavaScript, CSS — the entire
  frontend application layer. There is enormous, well-tested UX value here and
  no safety/performance reason to rewrite it.
- **Replace (port to Rust):** *all* Clojure and *all* Python — API server,
  query processor, ORM, drivers, sync, permissions, scheduler, embedding, and
  notification subsystems.
- **Integrate (do not reimplement):** Gaussian already ships **MCP Servers**
  and **NL2SQL** technologies. GaussAnalytics builds thin, well-typed
  **integration layers** to those services rather than reinventing them.

---

## 2. Why Rust

| Goal | How Rust delivers it |
|------|----------------------|
| **Performance** | No GC pauses; predictable latency; zero-cost abstractions; native async I/O via Tokio; the query compiler and row-streaming paths run with no JVM warmup and a fraction of the memory footprint. |
| **Security** | Memory safety by default eliminates whole bug classes (use-after-free, buffer overrun). A small, explicit dependency tree is auditable (`cargo audit`, `cargo deny`). Parameterized query generation is enforced by the type system. |
| **Operability** | Single statically-linked binary per service; trivial container images; fast cold start; first-class cross-compilation. |
| **Correctness** | Exhaustive `match`, `Result`-based error handling, and a strong type system make the query AST and permission checks hard to get wrong. |
| **Cost** | Lower memory/CPU per request → cheaper to run at scale; smaller images → faster autoscaling. |

We accept Rust's costs (slower iteration on some subsystems, a learning curve,
longer compiles) because the engine is long-lived infrastructure where safety
and steady-state performance dominate.

---

## 3. Strategic principles

1. **Strangler-fig, not big-bang.** The Rust services are introduced behind a
   stable HTTP contract. The frontend's API client is the contract boundary;
   it should not need to know which language serves a route. This lets us cut
   over endpoint-by-endpoint and keep a runnable product at every step.

2. **Contract-first.** The HTTP/JSON API and the query AST (GQL) are specified
   independently of any implementation language so the reused frontend keeps
   working unchanged.

3. **Type-driven safety.** SQL is *generated*, never concatenated. Every
   compiled query yields a parameterized statement plus a typed parameter
   vector. Permission checks are values that must be discharged before a query
   runs — enforced structurally, not by convention.

4. **Integrate, don't reinvent (AI).** NL2SQL and MCP are Gaussian platform
   capabilities. GaussAnalytics owns the *integration*, *governance*, and
   *guardrails* (schema grounding, SQL validation, permission enforcement,
   audit) — not the models.

5. **Operator-first.** Administration is a first-class product, delivered both
   in the web UI and as a fast, scriptable **Ratatui TUI** for operators who
   live in the terminal.

6. **Everything verifiable.** Each phase ships compiling code with tests and
   CI. "Done" means `cargo test` green and the binary runs.

---

## 4. Target architecture (summary)

A Cargo workspace of focused crates, composing into a few deployable binaries.
See [`ARCHITECTURE.md`](./ARCHITECTURE.md) for the full diagram.

```
                    ┌─────────────────────────────────────────┐
   Reused React/TS  │            gauss-server (axum)            │
   frontend  ─────► │  HTTP/JSON API · static asset serving     │
                    └───────┬───────────────┬───────────────────┘
                            │               │
        ┌───────────────────┘               └──────────────────┐
        ▼                                                       ▼
  gauss-query (GQL→SQL)                            gauss-mcp-gateway ─► Gaussian MCP Servers
  gauss-db (app DB / repos)                        gauss-nl2sql      ─► Gaussian NL2SQL
  gauss-auth (RBAC / sessions)
  gauss-core (domain + GQL AST)
        ▲
        │
  gauss-tui (Ratatui)  ── operator administration & monitoring
  gaussctl (CLI)       ── serve / admin / migrate / version
```

---

## 5. Risk register (top items)

| Risk | Mitigation |
|------|------------|
| Query-engine semantics drift from the reference | Golden-file tests: GQL fixtures → expected SQL per dialect; differential testing against recorded results. |
| Driver breadth (many databases) | Prioritize Postgres/MySQL/SQLite first; abstract dialect behind a trait; add drivers per demand. |
| Frontend coupling to old API shapes | Freeze the API contract; add a compatibility test suite that exercises the reused client against the Rust server. |
| AI guardrails (NL2SQL hallucination, MCP tool abuse) | All generated SQL is parsed/validated and runs under the requesting user's permissions; MCP calls are policy-gated and audited. |
| Network-restricted CI for Rust deps | Vendored/locked dependencies; lean dependency tree; reproducible `Cargo.lock`. |

---

## 6. Success criteria

- Feature parity for the **core analytics loop** (connect DB → sync schema →
  build question via GQL → visualize → dashboard → share/embed) on the Rust
  stack, with the reused frontend.
- p95 query-compile + dispatch latency materially below the reference engine;
  steady-state RSS a fraction of the JVM footprint.
- Zero `cargo audit` criticals; SBOM published per release.
- A TUI an operator can run to inspect databases, jobs, sessions, and health.
- NL2SQL and MCP available through governed integration layers with full audit.
