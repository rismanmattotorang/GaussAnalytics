# Web UI assessment: GaussAnalytics vs. Metabase

> **Owner:** Gaussian Technologies · **Status:** honest, living scorecard

This is a frank, feature-by-feature comparison of the GaussAnalytics web UI
against Metabase, the category-leading open-source BI tool. The goal is an
accurate picture — **not** a claim of total parity. Where GaussAnalytics is
behind on UI breadth, it is recorded as such; where it is ahead (almost always
on the **backend/engine** that powers the UI), that is noted too.

Legend: ✅ covered · 🟡 partial · ⬜ not yet.

## Core analytics loop

| Capability | Metabase | GaussAnalytics | Backend edge (Rust) |
|---|---|---|---|
| Connect a database | ✅ | ✅ (register + sync via API/UI) | Drivers as a trait; parameterized everywhere |
| Browse schema / tables | ✅ | ✅ (synced tables + fields, semantic types) | Sync **fingerprints** columns + infers semantic type |
| Visual query builder | ✅ rich | 🟡 source/table/fields, filter, summarize + group-by, limit | Builds **GQL**, compiled to bound SQL |
| Native SQL editor | ✅ (allows writes/DDL) | ✅ **read-only-guarded** editor, **`{{variables}}`**, **schema browser + snippets** | Writes/DDL/batching rejected *before* the DB; variables are bound params; cached |
| Run + view results table | ✅ | ✅ | Async streaming-ready, cached |
| Visualizations | ✅ many (bar/line/area/pie/map/funnel/pivot/…) | 🟡 table, **bar, line, area, funnel, scatter, combo, pie, pivot** (map pending) | — |
| Pivot tables | ✅ | ✅ pivot view on 3-column aggregates | Pure client transform over cached results |
| Save questions | ✅ | ✅ | Persisted via generic content store |
| Dashboards | ✅ drag-and-drop layout, filters, params, tabs | ✅ compose + **shared filters** + **drag layout** + **cross-filter** + **auto-refresh** + **links** + **tabs** | Filters become **bound GQL predicates** → parameterized SQL, permission-checked, cached |
| Dashboard subscriptions/alerts | ✅ | 🟡 **query alerts** via scheduler + **webhook/Slack** notifier (email/SMTP pending) | Lean Tokio scheduler (no Quartz) |

## AI / agentic

| Capability | Metabase | GaussAnalytics | Edge |
|---|---|---|---|
| Natural-language → SQL | ✅ (Metabot) | ✅ panel, **multi-turn** history | **Governed**: schema-grounded, read-only-guardrailed, runs under user perms |
| Agentic tool use (MCP) | ⬜ (not a core MB feature) | ✅ MCP gateway + **chained workflow** endpoint | Policy allow-list + full audit |

## Sharing / embedding

| Capability | Metabase | GaussAnalytics | Edge |
|---|---|---|---|
| Public links | ✅ | 🟡 (embed tokens cover this pattern) | — |
| Signed embedding | ✅ | ✅ HMAC-SHA256 tokens | Stateless, constant-time verified |
| Content export/import | ✅ (serialization) | ✅ portable JSON bundles | One generic content store |

## Administration / governance

| Capability | Metabase | GaussAnalytics | Edge |
|---|---|---|---|
| Users & roles | ✅ | ✅ users + value-based permissions | Permission discharge is a typed step |
| Persisted, scoped permissions | ✅ | ✅ per-user/per-database grants | — |
| API keys | ✅ | ✅ rotatable, SHA-256 hashed | — |
| Mandatory auth | ✅ | ✅ `require_auth` middleware | — |
| Terminal admin console | ⬜ | ✅ Ratatui TUI over the same API | Unique to GaussAnalytics |
| Usage analytics | ✅ (enterprise) | ✅ `GET /api/usage` (queries/cards/dashboards/users) | — |
| Row-level security | ✅ (enterprise) | ✅ admin-defined policies (`/api/rls`), enforced for non-admins | Injected as **bound GQL predicates** — parameterized SQL, unbypassable by query text |
| Metrics layer | ✅ | ✅ named reusable measures (`/api/metrics`, run on demand) | Stored as GQL aggregate queries; compiled to bound SQL |
| Audit logging | ✅ (enterprise) | 🟡 MCP/AI audit hooks | — |

## Where GaussAnalytics is *structurally* ahead

These are independent of the UI and benefit every screen:

- **Engine in Rust**: parameterized SQL by construction (SQL injection is
  impossible, not mitigated); GQL→SQL compiles at **~500k queries/sec/core**.
- **No GC / single static binary**: predictable latency, small footprint, fast
  cold start vs. a JVM deployment.
- **Governed AI by default**: NL2SQL and MCP run under the caller's permissions
  with audit — not bolted on.
- **Operability**: a fast terminal admin console in addition to the web UI.

## Honest gaps (tracked in `ROADMAP.md`)

1. **Visualization breadth** — geographic **map** charts (bar/line/area/funnel/
   scatter/**combo**/pie/pivot now exist).
2. **Dashboard polish** — nested/cross-dashboard drill paths (compose, shared
   filters, drag-and-drop layout, cross-filtering, auto-refresh, links, and
   **tabs** now exist).
3. **SQL editor polish** — inline autocomplete (bound `{{variables}}`, a
   **schema browser**, and **snippets** now exist).
4. **Subscription delivery** — **webhook/Slack** channel exists; native SMTP +
   a schedules UI remain (email is reachable today via an HTTP email API webhook).
5. **Driver breadth** — Postgres/MySQL/SQLite ✅ + BigQuery/Snowflake/ClickHouse
   (dialects golden-tested; REST drivers integration-stage, live-test pending).
   Further long tail (Redshift, Databricks, …) per demand.
6. **Metrics layer** ✅ (named measures via `/api/metrics`) and **row-level
   security** ✅ (admin policies via `/api/rls`, enforced as bound predicates for
   non-admins) now exist at the API + engine level. Remaining here:
   **models-as-virtual-datasets**, a dedicated **RLS/metrics management UI**, and
   **content versioning**.

## Bottom line

GaussAnalytics covers the **core BI loop end-to-end** (connect → explore →
visualize → save → dashboard → embed → alert → export) on a backend that is
materially faster, safer, and more governable than Metabase's. It does **not
yet** match Metabase's *visualization and dashboard-editing breadth* or its long
tail of enterprise UI features. Closing that UI breadth — on top of the superior
Rust engine — is the explicit focus of the remaining roadmap.
