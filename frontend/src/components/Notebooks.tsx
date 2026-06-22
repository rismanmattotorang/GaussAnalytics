import { useEffect, useState } from "react";
import {
  api,
  type CellKind,
  type CellOutput,
  type Dashboard,
  type Database,
  type Notebook,
  type NotebookCell,
  type QueryResult,
} from "../api/client";
import { ResultView } from "./ResultView";

/** Minimal, escaped Markdown → HTML (headings, bold, inline code, line breaks).
 * Input is escaped first, so the result is safe to inject. Mirrors the renderer
 * used for dashboard text cards. */
function mdToHtml(src: string): string {
  const esc = src.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  return esc
    .replace(/^### (.*)$/gm, "<h4>$1</h4>")
    .replace(/^## (.*)$/gm, "<h3>$1</h3>")
    .replace(/^# (.*)$/gm, "<h2>$1</h2>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\n/g, "<br>");
}

function newCell(kind: CellKind): NotebookCell {
  return { id: crypto.randomUUID(), kind, source: "" };
}

const KIND_LABEL: Record<CellKind, string> = {
  markdown: "Markdown",
  python: "Python",
  sql: "SQL",
  nl2sql: "NL2SQL",
  input: "Input",
  chart: "Chart",
  bignumber: "Big number",
};

function cellPlaceholder(kind: CellKind): string {
  switch (kind) {
    case "python":
      return "Python…";
    case "markdown":
      return "Markdown…";
    case "sql":
      return "SELECT … (read-only)";
    case "nl2sql":
      return "Ask in plain English…";
    case "input":
      return "value";
    default:
      return "";
  }
}

/** A single headline value taken from the first cell of a DataFrame. */
function BigNumber({ result }: { result: QueryResult }) {
  const value = result.rows[0]?.[0];
  const label = result.columns[0] ?? "";
  return (
    <div className="big-number">
      <div className="big-number__value">
        {value === null || value === undefined ? "∅" : String(value)}
      </div>
      <div className="big-number__label">{label}</div>
    </div>
  );
}

/** A compact preview table for a SQL/NL2SQL cell result (first rows). */
function PreviewTable({ result }: { result: QueryResult }) {
  const rows = result.rows.slice(0, 50);
  return (
    <div className="nb-preview">
      <table className="data-table">
        <thead>
          <tr>
            {result.columns.map((c) => (
              <th key={c}>{c}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, i) => (
            <tr key={i}>
              {row.map((cell, j) => (
                <td key={j}>{cell === null ? "∅" : String(cell)}</td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      <span className="muted">
        {result.rows.length} row{result.rows.length === 1 ? "" : "s"}
        {result.rows.length > rows.length ? ` (showing ${rows.length})` : ""}
      </span>
    </div>
  );
}

/** Render one normalized kernel output (stream text, MIME bundle, or error). */
function OutputView({ out }: { out: CellOutput }) {
  if (out.kind === "stream") {
    return <pre className="nb-output" data-stream={out.name}>{out.text}</pre>;
  }
  if (out.kind === "error") {
    const text = out.traceback.length ? out.traceback.join("\n") : `${out.ename}: ${out.evalue}`;
    return <pre className="nb-output nb-output--error">{text}</pre>;
  }
  const data = out.data;
  const png = data["image/png"];
  if (typeof png === "string") {
    return <img className="nb-output__img" alt="cell output" src={`data:image/png;base64,${png}`} />;
  }
  const html = data["text/html"];
  if (typeof html === "string") {
    return <div className="nb-output" dangerouslySetInnerHTML={{ __html: html }} />;
  }
  const plain = data["text/plain"];
  return <pre className="nb-output">{typeof plain === "string" ? plain : JSON.stringify(data)}</pre>;
}

interface CellMeta {
  sql?: string;
  preview?: QueryResult;
}

export function Notebooks({
  token,
  databases,
}: {
  token: string | null;
  databases: Database[];
}) {
  const [notebooks, setNotebooks] = useState<Notebook[]>([]);
  const [dashboards, setDashboards] = useState<Dashboard[]>([]);
  const [open, setOpen] = useState<Notebook | null>(null);
  const [name, setName] = useState("");
  const [cells, setCells] = useState<NotebookCell[]>([]);
  const [outputs, setOutputs] = useState<Record<string, CellOutput[]>>({});
  const [meta, setMeta] = useState<Record<string, CellMeta>>({});
  const [running, setRunning] = useState<string | null>(null);
  const [runningAll, setRunningAll] = useState(false);
  const [kernelRunning, setKernelRunning] = useState(false);
  const [saved, setSaved] = useState(false);
  const [createName, setCreateName] = useState("");
  const [publishTarget, setPublishTarget] = useState("");
  const [publishedId, setPublishedId] = useState<string | null>(null);
  const [aiPrompt, setAiPrompt] = useState("");
  const [aiDb, setAiDb] = useState("");
  const [assisting, setAssisting] = useState(false);
  const [mode, setMode] = useState("local");
  const [error, setError] = useState<string | null>(null);

  function load() {
    api
      .notebooks()
      .then(setNotebooks)
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
    api
      .dashboards()
      .then(setDashboards)
      .catch(() => {});
    api
      .notebookCapabilities()
      .then((c) => setMode(c.mode))
      .catch(() => {});
  }
  useEffect(load, []);

  function fail(e: unknown) {
    setError(e instanceof Error ? e.message : String(e));
  }

  function openNotebook(nb: Notebook) {
    setOpen(nb);
    setName(nb.name);
    setCells(nb.cells);
    setOutputs({});
    setMeta({});
    setKernelRunning(false);
    setError(null);
  }

  async function create() {
    if (!token || !createName) return;
    setError(null);
    try {
      const nb = await api.createNotebook({ name: createName, cells: [] }, token);
      setCreateName("");
      load();
      openNotebook(nb);
    } catch (e) {
      fail(e);
    }
  }

  async function save() {
    if (!token || !open) return;
    setError(null);
    setSaved(false);
    try {
      const nb = await api.updateNotebook(open.id, { name, cells }, token);
      setOpen(nb);
      setSaved(true);
      load();
    } catch (e) {
      fail(e);
    }
  }

  async function remove(id: string) {
    if (!token) return;
    try {
      await api.deleteNotebook(id, token);
      if (open?.id === id) setOpen(null);
      load();
    } catch (e) {
      fail(e);
    }
  }

  function patchCell(id: string, patch: Partial<NotebookCell>) {
    setCells((cs) => cs.map((c) => (c.id === id ? { ...c, ...patch } : c)));
  }

  function addCell(kind: CellKind) {
    setCells((cs) => [...cs, newCell(kind)]);
  }

  function removeCell(id: string) {
    setCells((cs) => cs.filter((c) => c.id !== id));
  }

  function moveCell(index: number, delta: number) {
    setCells((cs) => {
      const next = [...cs];
      const j = index + delta;
      if (j < 0 || j >= next.length) return cs;
      [next[index], next[j]] = [next[j], next[index]];
      return next;
    });
  }

  // Run a single cell. Returns false if it errored (so a "run all" sweep can
  // stop). Markdown cells are not executed.
  async function runCell(cell: NotebookCell): Promise<boolean> {
    if (!token || !open || cell.kind === "markdown") return true;
    setRunning(cell.id);
    try {
      const res = await api.runCell(open.id, cell, token);
      setOutputs((o) => ({ ...o, [cell.id]: res.outputs }));
      setMeta((m) => ({ ...m, [cell.id]: { sql: res.sql, preview: res.preview } }));
      setKernelRunning(true);
      return !res.outputs.some((o) => o.kind === "error");
    } catch (e) {
      fail(e);
      return false;
    } finally {
      setRunning(null);
    }
  }

  // Run cells in a given dependency order (ids), stopping on the first error.
  async function runSequence(order: string[]) {
    const byId = new Map(cells.map((c) => [c.id, c]));
    for (const id of order) {
      const cell = byId.get(id);
      if (!cell) continue;
      const ok = await runCell(cell);
      if (!ok) break;
    }
  }

  // Reactive re-run of the whole notebook, in dependency (topological) order
  // computed server-side. A cycle surfaces as an error.
  async function runAll() {
    if (!token || !open) return;
    setError(null);
    setRunningAll(true);
    try {
      const { order } = await api.runOrder(open.id, cells, null, token);
      await runSequence(order);
    } catch (e) {
      fail(e);
    } finally {
      setRunningAll(false);
    }
  }

  // Re-run a changed cell and only its transitive dependents (the minimal
  // reactive update) — e.g. tweak an Input, then recompute what depends on it.
  async function runDownstream(cell: NotebookCell) {
    if (!token || !open) return;
    setError(null);
    setRunningAll(true);
    try {
      const { order } = await api.runOrder(open.id, cells, cell.id, token);
      await runSequence(order);
    } catch (e) {
      fail(e);
    } finally {
      setRunningAll(false);
    }
  }

  // Pin a cell's output onto the selected dashboard as a tile (runs the
  // notebook server-side to snapshot the cell).
  async function publish(cell: NotebookCell) {
    if (!token || !open || !publishTarget) return;
    setError(null);
    setPublishedId(null);
    try {
      await api.publishCell(open.id, { cell_id: cell.id, dashboard_id: publishTarget }, token);
      setPublishedId(cell.id);
    } catch (e) {
      fail(e);
    }
  }

  // Export the open notebook as a downloadable .ipynb file.
  async function exportNotebook() {
    if (!open) return;
    setError(null);
    try {
      const doc = await api.exportNotebook(open.id);
      const blob = new Blob([JSON.stringify(doc, null, 2)], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `${open.name || "notebook"}.ipynb`;
      a.click();
      // Defer revocation so the browser has initiated the download first.
      setTimeout(() => URL.revokeObjectURL(url), 0);
    } catch (e) {
      fail(e);
    }
  }

  // Create a notebook from an uploaded .ipynb file, then open it.
  async function importNotebook(file: File) {
    if (!token) return;
    setError(null);
    try {
      const ipynb = JSON.parse(await file.text());
      const nb = await api.importNotebook({ ipynb }, token);
      load();
      openNotebook(nb);
    } catch (e) {
      fail(e);
    }
  }

  // Ask the in-notebook assistant to propose a cell, then append it.
  async function askAi() {
    if (!token || !open || !aiPrompt.trim()) return;
    setError(null);
    setAssisting(true);
    try {
      const res = await api.assistNotebook(
        open.id,
        { prompt: aiPrompt, database_id: aiDb || undefined },
        token,
      );
      setCells((cs) => [...cs, res.cell]);
      setAiPrompt("");
    } catch (e) {
      fail(e);
    } finally {
      setAssisting(false);
    }
  }

  async function startKernel() {
    if (!token || !open) return;
    try {
      await api.startKernel(open.id, token);
      setKernelRunning(true);
    } catch (e) {
      fail(e);
    }
  }

  async function stopKernel() {
    if (!token || !open) return;
    try {
      await api.stopKernel(open.id, token);
      setKernelRunning(false);
    } catch (e) {
      fail(e);
    }
  }

  if (!token) {
    return (
      <div className="notebooks">
        <h2>Notebooks</h2>
        <p className="muted">Sign in to create and run notebooks.</p>
      </div>
    );
  }

  if (open) {
    return (
      <div className="notebooks">
        <div className="notebooks__bar">
          <button className="link" onClick={() => setOpen(null)}>
            ← All notebooks
          </button>
          <input
            className="notebooks__title"
            value={name}
            onChange={(e) => setName(e.target.value)}
            aria-label="notebook name"
          />
          <span className="notebooks__actions">
            <button onClick={runAll} disabled={runningAll}>
              {runningAll ? "Running…" : "Run all"}
            </button>
            <button onClick={save}>Save</button>
            {saved && <span className="ds-ok">Saved</span>}
            <button className="link" onClick={exportNotebook}>
              Export .ipynb
            </button>
            {dashboards.length > 0 && (
              <select
                aria-label="publish target"
                value={publishTarget}
                onChange={(e) => setPublishTarget(e.target.value)}
                title="Dashboard to publish cells to"
              >
                <option value="">publish to…</option>
                {dashboards.map((d) => (
                  <option key={d.id} value={d.id}>
                    {d.name}
                  </option>
                ))}
              </select>
            )}
            {kernelRunning ? (
              <button className="link" onClick={stopKernel}>
                Stop kernel
              </button>
            ) : (
              <button className="link" onClick={startKernel}>
                Start kernel
              </button>
            )}
          </span>
        </div>

        {error && <p className="app__error">{error}</p>}

        <div className="notebooks__ai">
          <input
            placeholder="Ask AI to draft a cell (guardrailed SQL against the chosen source, else Python)…"
            value={aiPrompt}
            onChange={(e) => setAiPrompt(e.target.value)}
            aria-label="ask ai"
          />
          {databases.length > 0 && (
            <select
              aria-label="ai data source"
              value={aiDb}
              onChange={(e) => setAiDb(e.target.value)}
              title="Data source for a SQL suggestion (none → Python)"
            >
              <option value="">no source → Python</option>
              {databases.map((d) => (
                <option key={d.id} value={d.id}>
                  {d.name}
                </option>
              ))}
            </select>
          )}
          <button onClick={askAi} disabled={assisting || !aiPrompt.trim()}>
            {assisting ? "Thinking…" : "Ask AI"}
          </button>
        </div>

        {cells.length === 0 && <p className="muted">Empty notebook — add a cell below.</p>}

        {cells.map((cell, i) => {
          const isData = cell.kind === "sql" || cell.kind === "nl2sql";
          const usesVar = cell.kind === "chart" || cell.kind === "bignumber";
          const hasEditor =
            cell.kind === "python" ||
            cell.kind === "sql" ||
            cell.kind === "nl2sql" ||
            cell.kind === "markdown";
          const cellMeta = meta[cell.id];
          const cellOutputs = outputs[cell.id] ?? [];
          // When a structured preview is present it replaces the echoed repr;
          // still surface any errors from injection/fetch.
          const shownOutputs = cellMeta?.preview
            ? cellOutputs.filter((o) => o.kind === "error")
            : cellOutputs;
          return (
            <div key={cell.id} className="nb-cell" data-kind={cell.kind}>
              <div className="nb-cell__head">
                <span className="nb-cell__kind">{KIND_LABEL[cell.kind]}</span>
                <span className="nb-cell__tools">
                  <button className="link" onClick={() => moveCell(i, -1)} disabled={i === 0}>
                    ↑
                  </button>
                  <button
                    className="link"
                    onClick={() => moveCell(i, 1)}
                    disabled={i === cells.length - 1}
                  >
                    ↓
                  </button>
                  {cell.kind !== "markdown" && (
                    <>
                      <button onClick={() => runCell(cell)} disabled={running === cell.id}>
                        {running === cell.id ? "Running…" : "Run"}
                      </button>
                      <button
                        className="link"
                        title="Run this cell and everything that depends on it"
                        onClick={() => runDownstream(cell)}
                        disabled={runningAll}
                      >
                        Run ↓
                      </button>
                      {publishTarget && (
                        <button
                          className="link"
                          title="Pin this cell's output to the selected dashboard"
                          onClick={() => publish(cell)}
                        >
                          {publishedId === cell.id ? "Published ✓" : "Publish"}
                        </button>
                      )}
                    </>
                  )}
                  <button className="link" onClick={() => removeCell(cell.id)}>
                    Delete
                  </button>
                </span>
              </div>

              {/* Data cells: data source + output variable. */}
              {isData && (
                <div className="nb-cell__data-controls">
                  <select
                    aria-label="data source"
                    value={cell.database_id ?? ""}
                    onChange={(e) => patchCell(cell.id, { database_id: e.target.value || null })}
                  >
                    <option value="">data source…</option>
                    {databases.map((d) => (
                      <option key={d.id} value={d.id}>
                        {d.name}
                      </option>
                    ))}
                  </select>
                  <span className="muted">→</span>
                  <input
                    className="nb-cell__var"
                    placeholder="df"
                    aria-label="output variable"
                    value={cell.output_var ?? ""}
                    onChange={(e) => patchCell(cell.id, { output_var: e.target.value || null })}
                  />
                </div>
              )}

              {/* Chart / big-number cells: which DataFrame variable to read. */}
              {usesVar && (
                <div className="nb-cell__data-controls">
                  <span className="muted">DataFrame</span>
                  <input
                    className="nb-cell__var"
                    placeholder="df"
                    aria-label="dataframe variable"
                    value={cell.input_var ?? ""}
                    onChange={(e) => patchCell(cell.id, { input_var: e.target.value || null })}
                  />
                </div>
              )}

              {/* Input cells: variable name + value. */}
              {cell.kind === "input" && (
                <div className="nb-cell__input">
                  <input
                    className="nb-cell__var"
                    placeholder="variable"
                    aria-label="input variable"
                    value={cell.input_var ?? ""}
                    onChange={(e) => patchCell(cell.id, { input_var: e.target.value || null })}
                  />
                  <span className="muted">=</span>
                  <input
                    className="nb-cell__val"
                    placeholder={cellPlaceholder(cell.kind)}
                    aria-label="input value"
                    value={cell.source}
                    onChange={(e) => patchCell(cell.id, { source: e.target.value })}
                  />
                </div>
              )}

              {hasEditor && (
                <textarea
                  className="nb-cell__src native__editor"
                  value={cell.source}
                  spellCheck={false}
                  rows={Math.max(2, cell.source.split("\n").length)}
                  onChange={(e) => patchCell(cell.id, { source: e.target.value })}
                  placeholder={cellPlaceholder(cell.kind)}
                />
              )}

              {cell.kind === "markdown" && cell.source && (
                <div
                  className="nb-cell__md"
                  dangerouslySetInnerHTML={{ __html: mdToHtml(cell.source) }}
                />
              )}

              {/* NL2SQL shows the SQL it generated. */}
              {cell.kind === "nl2sql" && cellMeta?.sql && (
                <pre className="nb-cell__sql">{cellMeta.sql}</pre>
              )}

              {/* Preview rendering by kind: a nivo chart, a headline number, or
                  a table for SQL/NL2SQL data cells. */}
              {cellMeta?.preview &&
                (cell.kind === "chart" ? (
                  <ResultView result={cellMeta.preview} />
                ) : cell.kind === "bignumber" ? (
                  <BigNumber result={cellMeta.preview} />
                ) : (
                  <PreviewTable result={cellMeta.preview} />
                ))}

              {shownOutputs.map((out, k) => (
                <OutputView key={k} out={out} />
              ))}
            </div>
          );
        })}

        <div className="notebooks__add">
          <button className="link" onClick={() => addCell("python")}>
            + Python
          </button>
          <button className="link" onClick={() => addCell("sql")}>
            + SQL
          </button>
          <button className="link" onClick={() => addCell("nl2sql")}>
            + NL2SQL
          </button>
          <button className="link" onClick={() => addCell("input")}>
            + Input
          </button>
          <button className="link" onClick={() => addCell("chart")}>
            + Chart
          </button>
          <button className="link" onClick={() => addCell("bignumber")}>
            + Big number
          </button>
          <button className="link" onClick={() => addCell("markdown")}>
            + Markdown
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="notebooks">
      <h2>Notebooks</h2>
      <p className="muted">
        Mix Markdown notes, SQL/NL2SQL queries (results land as a pandas
        DataFrame), inputs, Python, and nivo charts / big numbers over any
        DataFrame. <strong>Run all</strong> executes in dependency order;{" "}
        <strong>Run ↓</strong> re-runs a cell and everything that depends on it.
        Import/export <code>.ipynb</code> and publish cells to dashboards. Code
        runs {mode === "managed" ? "on a managed sandboxed kernel" : "on your local Jupyter kernel"}{" "}
        and requires <code>GAUSS_JUPYTER_ENABLED</code>.
      </p>

      {error && <p className="app__error">{error}</p>}

      <div className="notebooks__create">
        <input
          placeholder="New notebook name"
          value={createName}
          onChange={(e) => setCreateName(e.target.value)}
        />
        <button onClick={create} disabled={!createName}>
          Create
        </button>
        <label className="link notebooks__import">
          Import .ipynb
          <input
            type="file"
            accept=".ipynb,application/json"
            style={{ display: "none" }}
            onChange={(e) => {
              const file = e.target.files?.[0];
              if (file) importNotebook(file);
              e.target.value = "";
            }}
          />
        </label>
      </div>

      {notebooks.length === 0 ? (
        <p className="muted">No notebooks yet.</p>
      ) : (
        <ul className="notebooks__list">
          {notebooks.map((nb) => (
            <li key={nb.id}>
              <button className="link" onClick={() => openNotebook(nb)}>
                {nb.name}
              </button>
              <span className="muted">{nb.cells.length} cells</span>
              <button className="link" onClick={() => remove(nb.id)}>
                delete
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
