import { useEffect, useState } from "react";
import {
  api,
  type CellKind,
  type CellOutput,
  type Notebook,
  type NotebookCell,
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

/** Render one normalized kernel output (stream text, MIME bundle, or error). */
function OutputView({ out }: { out: CellOutput }) {
  if (out.kind === "stream") {
    return <pre className="nb-output" data-stream={out.name}>{out.text}</pre>;
  }
  if (out.kind === "error") {
    const text = out.traceback.length ? out.traceback.join("\n") : `${out.ename}: ${out.evalue}`;
    return <pre className="nb-output nb-output--error">{text}</pre>;
  }
  // data: prefer an image, then HTML, then plain text.
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

export function Notebooks({ token }: { token: string | null }) {
  const [notebooks, setNotebooks] = useState<Notebook[]>([]);
  const [open, setOpen] = useState<Notebook | null>(null);
  const [name, setName] = useState("");
  const [cells, setCells] = useState<NotebookCell[]>([]);
  const [outputs, setOutputs] = useState<Record<string, CellOutput[]>>({});
  const [running, setRunning] = useState<string | null>(null);
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

  function updateCell(id: string, source: string) {
    setCells((cs) => cs.map((c) => (c.id === id ? { ...c, source } : c)));
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

  async function runCell(cell: NotebookCell) {
    if (!token || !open) return;
    setError(null);
    setRunning(cell.id);
    try {
      const res = await api.runCell(open.id, cell.source, token);
      setOutputs((o) => ({ ...o, [cell.id]: res.outputs }));
      setKernelRunning(true);
    } catch (e) {
      fail(e);
    } finally {
      setRunning(null);
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

        {cells.length === 0 && (
          <p className="muted">Empty notebook — add a cell below.</p>
        )}

        {cells.map((cell, i) => (
          <div key={cell.id} className="nb-cell" data-kind={cell.kind}>
            <div className="nb-cell__head">
              <span className="nb-cell__kind">{cell.kind}</span>
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
                {cell.kind === "python" && (
                  <button onClick={() => runCell(cell)} disabled={running === cell.id}>
                    {running === cell.id ? "Running…" : "Run"}
                  </button>
                )}
                <button className="link" onClick={() => removeCell(cell.id)}>
                  Delete
                </button>
              </span>
            </div>
            <textarea
              className="nb-cell__src native__editor"
              value={cell.source}
              spellCheck={false}
              rows={Math.max(2, cell.source.split("\n").length)}
              onChange={(e) => updateCell(cell.id, e.target.value)}
              placeholder={cell.kind === "python" ? "Python…" : "Markdown…"}
            />
            {cell.kind === "markdown" && cell.source && (
              <div
                className="nb-cell__md"
                dangerouslySetInnerHTML={{ __html: mdToHtml(cell.source) }}
              />
            )}
            {(outputs[cell.id] ?? []).map((out, k) => (
              <OutputView key={k} out={out} />
            ))}
          </div>
        ))}

        <div className="notebooks__add">
          <button className="link" onClick={() => addCell("python")}>
            + Python cell
          </button>
          <button className="link" onClick={() => addCell("markdown")}>
            + Markdown cell
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="notebooks">
      <h2>Notebooks</h2>
      <p className="muted">
        Mix Markdown notes with Python that runs on your local Jupyter kernel.
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
