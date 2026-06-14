# ADR 0001 — Rust as the backend stack

- **Status:** Accepted
- **Date:** 2026-06-14
- **Owner:** Gaussian Technologies

## Context

The reference analytics platform's backend is Clojure (JVM) with some Python in
the AI path. We want better steady-state performance, a smaller and auditable
security surface, predictable latency (no GC pauses), single-binary
deployment, and strong compile-time guarantees for the query engine and
permission system.

## Decision

Port all backend Clojure and Python to a **Rust** workspace. Use:
- `tokio` for async runtime, `axum` + `tower-http` for HTTP.
- `serde`/`serde_json` for the JSON API contract (kept compatible with the
  reused frontend).
- `sqlx` for the application database (Phase 2) behind repository traits.
- `argon2` for password hashing.
- `ratatui` + `crossterm` for the administration TUI.

The frontend (TypeScript/JavaScript/CSS) is **reused** unchanged behind a
stable HTTP/JSON contract.

## Consequences

- **+** Memory safety, no GC, small images, fast cold start, auditable deps.
- **+** Type system enforces parameterized SQL and permission discharge.
- **−** Longer compile times and a steeper learning curve than Clojure.
- **−** Some libraries (e.g., breadth of JDBC drivers) must be rebuilt or
  sourced; mitigated by phasing drivers in by demand.
- We adopt a strangler-fig migration so the product stays runnable throughout.
