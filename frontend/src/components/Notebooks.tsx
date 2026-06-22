import { useEffect, useState } from "react";
import {
  api,
  type CellKind,
  type CellOutput,
  type Database,
  type Notebook,
  type NotebookCell,
  type QueryResult,
} from "../api/client";

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
  }
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
  const [error, setError] = useState<string | null>(null);

  function load() {
    api
      .notebooks()
      .then(setNotebooks)
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
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

  // Reactive re-run: execute every cell top-to-bottom. Inputs are re-injected
  // and downstream SQL/Python recompute with the new values. Stops on error.
  async function runAll() {
    if (!token || !open) return;
    setError(null);
    setRunningAll(true);
    try {
      for (const cell of cells) {
        const ok = await runCell(cell);
        if (!ok) break;
      }
    } finally {
      setRunningAll(false);
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

        {cells.length === 0 && <p className="muted">Empty notebook — add a cell below.</p>}

        {cells.map((cell, i) => {
          const isData = cell.kind === "sql" || cell.kind === "nl2sql";
          const cellMeta = meta[cell.id];
          const cellOutputs = outputs[cell.id] ?? [];
          // For data cells the structured preview replaces the echoed repr;
          // still surface any errors from injection.
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
                    <button onClick={() => runCell(cell)} disabled={running === cell.id}>
                      {running === cell.id ? "Running…" : "Run"}
                    </button>
                  )}
                  <button className="link" onClick={() => removeCell(cell.id)}>
                    Delete
                  </button>
                </span>
              </div>

              {/* Data cells: source-and-variable controls. */}
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

              {/* Input cells: variable name + value. */}
              {cell.kind === "input" ? (
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
              ) : (
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

              {cellMeta?.preview && <PreviewTable result={cellMeta.preview} />}

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
        DataFrame), inputs, and Python that runs on your local Jupyter kernel.
        Code execution requires <code>GAUSS_JUPYTER_ENABLED</code> and a running
        Jupyter Server.
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
