# GaussAnalytics Notebooks — Design & Delivery Plan

> **Status:** N0–N3 delivered · N4–N5 proposed (post-1.0 initiative) · **Owner:** Gaussian Technologies
> **Theme:** bring a [Deepnote](https://github.com/deepnote/deepnote)-class data
> notebook into GaussAnalytics — NL2SQL, data preprocessing, ML/DL — and wire its
> outputs into dashboards.

## 1. Goal & user scenarios

Add an embedded, block-based **data notebook** to GaussAnalytics so an analyst can,
in one place:

1. **Query** — write SQL or ask in natural language (**NL2SQL**); results land as a
   pandas `DataFrame`.
2. **Preprocess** — clean/reshape/join data in Python (pandas/polars) with reactive
   re-execution.
3. **Model** — train/evaluate ML/DL (scikit-learn, XGBoost, PyTorch, …) and inspect
   metrics and plots inline.
4. **Publish** — pin a block's output (chart, table, big-number, model score) onto a
   GaussAnalytics **dashboard**, and refresh it on a schedule.

**Execution model (per the brief): bring-your-own local runtime.** The user installs
**Python + Jupyter** locally alongside GaussAnalytics. GaussAnalytics does **not**
re-implement a kernel; it embeds the notebook **UX** and orchestrates execution
against the user's local **Jupyter Server**. This keeps GaussAnalytics' "secure,
auditable, single-binary" posture intact (no embedded Python, no arbitrary code in
the Rust process) while giving users the full local scientific-Python stack.

## 2. What we adopt from Deepnote (and what we don't)

Deepnote open-sourced its **block model** and **converters** as MIT TypeScript
packages — we reuse them instead of inventing a format:

| Adopt | How |
|---|---|
| `@deepnote/blocks` — the block schema (Code, SQL, Text, Markdown, Input, Visualization, Button, Big Number, Image, Separator) + validation | Frontend dependency; the canonical block types our model mirrors |
| `@deepnote/convert` — bidirectional `.deepnote` ⇄ Jupyter `.ipynb`/Quarto | Import/export + interop with users' existing notebooks |
| `.deepnote` **YAML** document format | Version-control-friendly notebook persistence (vs Jupyter's JSON) |
| **Reactive execution** (re-run dependents when an input/SQL result changes) | A block dependency DAG in `gauss-notebook` |
| **SQL-block → DataFrame** ergonomics | Our SQL/NL2SQL blocks inject a `DataFrame` variable into the kernel |
| Point-and-click **chart blocks**, **input** widgets, **publish-as-app** | Mapped onto our existing nivo charts, dashboard cards, and scheduler |

We **do not** adopt: Deepnote's cloud runtime, real-time CRDT collaboration, or its
hosted agent. Those are later/optional (§9).

## 3. Architecture

```
React app ── Notebook UI (blocks, outputs, widgets)
   │   @deepnote/blocks (schema) · @deepnote/convert (ipynb) · kernel WS client
   ▼
gauss-server (axum)
   ├── notebook CRUD            → gauss-db ContentRepository (kind="notebook")
   ├── SQL / NL2SQL block run   → gauss-query · gauss-drivers (ConnectionRegistry) · gauss-textsql
   ├── publish block → dashboard→ gauss-db content (dashboard cards)
   └── kernel gateway (proxy)   ──WS/REST──►  user's local Jupyter Server
                                              (Python kernel: pandas, sklearn, torch…)
gauss-scheduler ── scheduled notebook runs → refresh published outputs
```

**Key decision — kernel via Jupyter Server, not raw ZMQ.** GaussAnalytics talks to
the local Jupyter Server's **REST + WebSocket** API (`/api/kernels`, channel
sockets) rather than the five-socket ZeroMQ wire protocol. This is dramatically
simpler and robust, and it is exactly the "user runs Jupyter locally" model the
brief asks for. Config: `GAUSS_JUPYTER_ENABLED`, `GAUSS_JUPYTER_URL`
(default `http://127.0.0.1:8888`), `GAUSS_JUPYTER_TOKEN`.

### New crate: `gauss-notebook`
- **Document model** — a `Notebook { id, name, blocks: Vec<Block>, … }` mirroring
  `@deepnote/blocks`; `Block` enum: `Code{lang,source}`, `Sql{source|nl_prompt, database_id, output_var}`, `Markdown`, `Input{kind,var,default}`, `Chart{df_var,spec}`, `BigNumber`, `Image`, `Separator`. Serde to the `.deepnote`-compatible JSON/YAML.
- **Kernel gateway client** — start/attach/interrupt/restart kernels; send
  `execute_request`; parse `iopub` messages (`stream`, `display_data`,
  `execute_result`, `error`, `status`) into a normalized `CellOutput` stream.
- **Reactive DAG** — track block dependencies (variable defs/uses + SQL outputs +
  inputs); on change, re-run downstream blocks in topological order; reject cycles.
- **SQL/NL2SQL runner** — compile/execute via `gauss-query`/`gauss-drivers` (honoring
  RLS + the v1.0 `ConnectionRegistry`), then **inject** the result into the kernel as
  a `DataFrame` (`output_var = pd.DataFrame.from_records(...)`), so Python blocks
  consume it. NL2SQL blocks first call `gauss-textsql` to produce guardrailed SQL.

### Server routes (new)
```
GET/POST   /api/notebooks                 list / create
GET/PUT/DELETE /api/notebooks/{id}        load / save / delete
POST       /api/notebooks/{id}/kernel     start or attach a kernel
DELETE     /api/notebooks/{id}/kernel     shut down
WS         /api/notebooks/{id}/channels   live kernel I/O (status + outputs)
POST       /api/notebooks/{id}/blocks/{block}/run   execute one block (streams outputs)
POST       /api/notebooks/{id}/sql        run a SQL/NL2SQL block → {rows, df_preview}
POST       /api/notebooks/{id}/publish    pin {block} as a dashboard card
```
Notebooks persist via the existing `ContentRepository` (`kind="notebook"`), inherit
permissions/`CreateContent`, and appear in export/import.

### Frontend
- `Notebooks.tsx` + per-block components; an output renderer for text/HTML/images
  (matplotlib/plotly PNG/SVG), DataFrame tables, errors (with tracebacks), and
  GaussAnalytics-native nivo charts.
- A small **kernel WebSocket client** (run/interrupt/restart, streamed outputs).
- Reuse `@deepnote/blocks` for types/validation and `@deepnote/convert` for
  `.ipynb` import/export. Charts: a chart block builds a spec from a `DataFrame`
  via the existing `gauss-insight`/`gauss-chart` recommender, rendered by nivo;
  kernel-produced matplotlib/plotly images render as-is.

## 4. Scenario → mechanism

| Scenario | Mechanism |
|---|---|
| **NL2SQL query** | NL2SQL block → `gauss-textsql` (schema-grounded, guardrailed) → SQL block → `DataFrame` |
| **Data preprocessing** | Python code blocks (pandas/polars) on the local kernel; reactive re-run |
| **ML / DL** | Python blocks using the user's local sklearn/xgboost/torch; metrics + plots captured as outputs; artifacts written to the project workspace |
| **Dashboard integration** | `publish` a block output as a dashboard card (`source: notebook_block { notebook_id, block_id }`); `gauss-scheduler` re-runs the notebook and refreshes pinned outputs |

## 5. Delivery phases

- **N0 — Spike ✅ delivered:** the `gauss-notebook` crate ships a **kernel gateway**
  that drives a local Jupyter Server over REST + WebSocket — start/shut down a kernel,
  send `execute_request`, and stream `iopub` outputs (`stream`/`execute_result`/
  `display_data`/`error`) into a normalized `CellOutput`, terminating on `status: idle`
  for the request's own `parent_header`. Proven by a **mock-Jupyter** test suite (axum
  REST mock + `tokio-tungstenite` WS mock) so CI stays green without Python. Wired to
  `GAUSS_JUPYTER_ENABLED` / `_URL` / `_TOKEN` (off by default). `@deepnote/blocks`/
  `convert` version pinning lands with the frontend in N1.
- **N1 — Notebook MVP ✅ delivered:** the `Notebook`/`NotebookCell` document model
  (in `gauss-core`) persists via the existing `ContentRepository` (`kind="notebook"`);
  `gauss-server` exposes notebook **CRUD** plus kernel control — `POST/DELETE
  /api/notebooks/{id}/kernel` (start/attach + shut down), `POST .../interrupt`, and
  `POST .../run` (execute a Python cell on the notebook's kernel, returning normalized
  outputs). A React **Notebooks** page edits ordered **Markdown + Python** cells, runs
  code, and renders stream/data/error outputs. Kernel control reuses the N0 gateway and
  stays behind `GAUSS_JUPYTER_ENABLED`: CRUD always works; run/kernel endpoints report
  the integration as disabled until an operator opts in. Reactive re-runs and streaming
  to the browser land in N2/N3.
- **N2 — Data ✅ delivered:** **SQL** and **NL2SQL** cells run through the governed
  query path (`ReadDatabase` permission + read-only guardrail + pooled connection +
  result cache) — NL2SQL is translated to guardrailed SQL grounded on the synced
  schema — and the result is **injected into the kernel as a pandas `DataFrame`**
  (named by `output_var`, default `df`), with an inline preview table. **Input** cells
  bind a typed variable (int/float/bool/str) into the kernel. A **Run all** sweep
  re-executes cells top-to-bottom, so changing an input and re-running recomputes
  downstream SQL/Python — reactive re-runs without (yet) a dependency DAG. Injection
  code is generated safely (validated identifiers; data shipped as in-kernel-parsed
  JSON). Note: policy-level **RLS** applies on the structured (GQL) path; raw-SQL and
  NL2SQL cells are permission-checked and read-only-guarded. The explicit dependency
  **DAG** (variable def/use analysis, cycle rejection) lands in N3.
- **N3 — Visuals & reactivity ✅ delivered:** **Chart** and **Big Number** cells
  render a kernel `DataFrame` (fetched as `{columns, rows}`) through the existing
  **nivo** stack — Chart cells reuse `ResultView` (bar/line/area/funnel/scatter/combo/
  pie + smart default + table fallback), Big Number shows a headline value.
  **matplotlib/plotly** pass through Python cells via `display_data` (image/PNG/SVG,
  HTML), rendered as-is. A real reactive **dependency DAG** lives in `gauss-notebook`
  (`dag.rs`): cells are reduced to define/use variable sets (SQL/Input define, Chart/
  BigNumber use, Python via a small heuristic), `topo_order` gives a safe run order and
  **rejects cycles**, and `downstream` gives the minimal re-run set. The server exposes
  `POST /api/notebooks/{id}/run-order`; the UI's **Run all** executes in topological
  order and **Run ↓** re-runs a cell plus its transitive dependents.
- **N4 — Dashboards & schedule:** publish block outputs as dashboard cards; scheduled
  notebook runs refresh them (`gauss-scheduler`).
- **N5 — Interop & scale:** `.ipynb` import/export (`@deepnote/convert`); in-notebook
  **AI agent** (reuse `gauss-engine` + tools); optional **server-side sandboxed
  kernels** (containers) for hosted/multi-user deployments.

Each phase ends green under the existing bar: `cargo fmt`/`clippy -D warnings`/
`cargo test`, frontend typecheck+test+build, tests for new behavior, and docs.

## 6. Testing strategy

- **Unit:** block model serde + `.deepnote` round-trip; DAG topo-order + cycle
  rejection; SQL→DataFrame-injection codegen; output normalization.
- **Gateway:** drive the kernel client against a **mock Jupyter Server** (WS
  fixture) asserting the execute/stream/error/interrupt contract — hermetic, no
  Python in CI.
- **Integration (gated):** a `#[ignore]`/feature-gated suite against a **real local
  Jupyter** (mirrors how the Postgres/Snowflake drivers are validated) covering
  Python exec, SQL→df, a sklearn fit, and publish-to-dashboard.
- **Frontend:** block rendering, output rendering, kernel-client reducer; a
  `.ipynb` import/export round-trip via `@deepnote/convert`.

## 7. Security & governance

- **Trust model:** kernels execute on the **user's machine** (local, user-owned) —
  GaussAnalytics never runs arbitrary code in its own process. Documented as the
  v1 model; **server-side execution requires the sandboxed-kernel phase (N5)** with
  per-user containers, resource limits, and egress policy.
- **Data access:** SQL/NL2SQL blocks reuse the governed path — driver
  `ConnectionRegistry`, **RLS**, read-only guardrails, audit. Connection secrets stay
  server-side (already masked in API responses).
- **Outputs:** size-capped/streamed; HTML/JS outputs sanitized before render.
- **Secrets in code:** notebooks are content (permissioned); guidance to keep
  credentials in GaussAnalytics data sources, not cell source.

## 8. Risks & mitigations

| Risk | Mitigation |
|---|---|
| Kernel protocol complexity | Use Jupyter Server REST/WS (not ZMQ); spike in N0 |
| Long-running / runaway cells | Per-cell timeout, interrupt, restart; output caps |
| Reactive cycles / surprise re-runs | Explicit DAG, cycle rejection, opt-in reactivity |
| `.deepnote` schema drift | Pin `@deepnote/blocks`; convert tests in CI |
| Multi-user server execution | Local-per-user now; sandboxed containers in N5 |
| Arbitrary-code security | Local trust model documented; sandbox gate before any hosted exec |

## 9. Open questions (for review)

1. **Runtime topology:** local Jupyter only for v1, or also a GaussAnalytics-managed
   sandboxed kernel option in the first release?
2. **DataFrame engine:** pandas-first (ubiquitous) with optional polars?
3. **Collaboration:** single-editor + autosave for v1, CRDT later — acceptable?
4. **Doc storage:** keep notebooks in the content store (simple, versioned with
   export) vs. files on disk (closer to `.deepnote` UX)?
5. **Scope of N1** for the first shippable PR (recommend: model + CRUD + Markdown/
   Python + gateway execute/stream).

## 10. First PR (proposed scope)

`gauss-notebook` crate (document model + `.deepnote` serde + a kernel-gateway client
with a **mock-Jupyter** test) + notebook CRUD routes on `gauss-server` + a minimal
React notebook (Markdown + Python blocks, run/stream/interrupt). No data/ML yet —
that lands in N2/N3 on this foundation. Everything behind `GAUSS_JUPYTER_ENABLED`
(off by default), so the build/CI stay green without Python.
