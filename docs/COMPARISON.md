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
| Scheduling | Quartz | **Lean Tokio scheduler** (deterministic, unit-tested) + query alerts | No heavyweight scheduler dependency |

## Performance posture (qualitative)

The hot paths — query compilation, parameter binding, row streaming — run with
no JVM warmup and a fraction of the memory footprint. The query compiler is
allocation-light and fully synchronous; I/O is async end-to-end on Tokio. A
single binary cold-starts in milliseconds. (Quantitative benchmarks land with a
dedicated `bench/` harness; the architecture is built so those numbers are
favorable by construction.)

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

- **Visualization library & dashboard editor**: the reused React frontend has
  the typed API client and shell; the full chart/dashboard UX is being ported.
- **Driver breadth**: SQLite/Postgres/MySQL today; the long tail (BigQuery,
  Snowflake, Redshift, etc.) is added per demand behind the `Driver` trait.
- **Content portability** (export/import), **row-level sandboxing**, and the
  full subscriptions/pulses delivery matrix are scheduled for Phase 4.
- Quantitative **benchmarks** and an SBOM/supply-chain pipeline are pending.

The thesis: match the product surface area over time, while being *structurally*
better on performance, safety, operability, and governed AI from day one.
