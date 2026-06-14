import { useEffect, useState } from "react";
import {
  api,
  type Card,
  type Dashboard,
  type DashboardCardResult,
  type DashboardParameter,
  type ParamBinding,
  type ParamKind,
} from "../api/client";
import { matchingParam, move, orderedLayout, type LayoutItem } from "../lib/dashboard";
import { ResultView } from "./ResultView";

function valuesObj(filters: Record<string, string>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(filters)) if (v !== "") out[k] = v;
  return out;
}

export function Dashboards({ token }: { token: string | null }) {
  const [dashboards, setDashboards] = useState<Dashboard[]>([]);
  const [cards, setCards] = useState<Card[]>([]);
  const [open, setOpen] = useState<Dashboard | null>(null);
  const [results, setResults] = useState<DashboardCardResult[]>([]);
  const [filterValues, setFilterValues] = useState<Record<string, string>>({});
  const [layout, setLayout] = useState<LayoutItem[]>([]);
  const [dragIndex, setDragIndex] = useState<number | null>(null);
  const [autoRefresh, setAutoRefresh] = useState<number>(0);
  const [error, setError] = useState<string | null>(null);

  // Create-form state.
  const [name, setName] = useState("");
  const [selected, setSelected] = useState<string[]>([]);
  const [params, setParams] = useState<DashboardParameter[]>([]);
  const [bindings, setBindings] = useState<ParamBinding[]>([]);
  const [links, setLinks] = useState<string[]>([]);

  function load() {
    Promise.all([api.dashboards(), api.cards()])
      .then(([d, c]) => {
        setDashboards(d);
        setCards(c);
      })
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  }
  useEffect(load, []);

  const cardTitle = (id: string) => cards.find((c) => c.id === id)?.name ?? id;
  const resultFor = (id: string) => results.find((r) => r.card_id === id);

  async function runBoard(d: Dashboard, values: Record<string, unknown>) {
    setError(null);
    try {
      setResults(await api.runDashboard(d.id, values));
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  async function openDash(d: Dashboard) {
    setOpen(d);
    setResults([]);
    setFilterValues({});
    setLayout(orderedLayout(d));
    setAutoRefresh(0);
    await runBoard(d, {});
  }

  // Auto-refresh timer.
  useEffect(() => {
    if (!open || autoRefresh <= 0) return;
    const handle = setInterval(() => runBoard(open, valuesObj(filterValues)), autoRefresh * 1000);
    return () => clearInterval(handle);
  }, [open, autoRefresh, filterValues]);

  function crossFilter(column: string, value: unknown) {
    if (!open) return;
    const param = matchingParam(open.parameters, column);
    if (!param) return;
    const next = { ...filterValues, [param]: String(value) };
    setFilterValues(next);
    runBoard(open, valuesObj(next));
  }

  function onDrop(to: number) {
    if (dragIndex === null) return;
    setLayout((l) => move(l, dragIndex, to));
    setDragIndex(null);
  }

  function toggleWidth(i: number) {
    setLayout((l) => l.map((x, j) => (j === i ? { ...x, w: x.w === 2 ? 1 : 2 } : x)));
  }

  async function saveLayout() {
    if (!token || !open) return;
    try {
      const updated = await api.updateDashboard(
        open.id,
        {
          name: open.name,
          card_ids: open.card_ids,
          parameters: open.parameters,
          bindings: open.bindings,
          layout: layout.map((l) => ({ card_id: l.card_id, w: l.w })),
          links: open.links,
        },
        token,
      );
      setOpen(updated);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  // --- create-form helpers ---
  function toggle(id: string) {
    setSelected((s) => (s.includes(id) ? s.filter((x) => x !== id) : [...s, id]));
  }
  function toggleLink(id: string) {
    setLinks((l) => (l.includes(id) ? l.filter((x) => x !== id) : [...l, id]));
  }
  function addParam() {
    setParams((p) => [...p, { name: "", kind: "text" }]);
  }
  function setParamField(i: number, patch: Partial<DashboardParameter>) {
    setParams((p) => p.map((x, j) => (j === i ? { ...x, ...patch } : x)));
  }
  function addBinding() {
    setBindings((b) => [
      ...b,
      { parameter: params[0]?.name ?? "", card_id: selected[0] ?? "", field: "", op: "eq" },
    ]);
  }
  function setBindingField(i: number, patch: Partial<ParamBinding>) {
    setBindings((b) => b.map((x, j) => (j === i ? { ...x, ...patch } : x)));
  }
  async function create() {
    if (!token || !name) return;
    setError(null);
    try {
      await api.createDashboard(
        { name, card_ids: selected, parameters: params, bindings, links },
        token,
      );
      setName("");
      setSelected([]);
      setParams([]);
      setBindings([]);
      setLinks([]);
      load();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  if (open) {
    return (
      <div className="dash">
        <button className="link" onClick={() => setOpen(null)}>
          ← all dashboards
        </button>
        <h2>{open.name}</h2>

        <div className="dash__bar">
          {(open.parameters ?? []).map((p) => (
            <label key={p.name}>
              {p.name}
              <input
                type={p.kind === "number" ? "number" : "text"}
                value={filterValues[p.name] ?? ""}
                onChange={(e) => setFilterValues((v) => ({ ...v, [p.name]: e.target.value }))}
              />
            </label>
          ))}
          {(open.parameters?.length ?? 0) > 0 && (
            <button onClick={() => runBoard(open, valuesObj(filterValues))}>Apply</button>
          )}
          <label>
            auto-refresh
            <select value={autoRefresh} onChange={(e) => setAutoRefresh(Number(e.target.value))}>
              <option value={0}>off</option>
              <option value={10}>10s</option>
              <option value={30}>30s</option>
              <option value={60}>60s</option>
            </select>
          </label>
          {token && (
            <button className="link" onClick={saveLayout}>
              save layout
            </button>
          )}
        </div>
        {(open.links ?? []).length > 0 && (
          <div className="dash__bar">
            <span className="muted">Linked:</span>
            {open.links!.map((id) => {
              const d = dashboards.find((x) => x.id === id);
              return (
                <button key={id} className="link" onClick={() => d && openDash(d)} disabled={!d}>
                  {d?.name ?? "(unknown)"}
                </button>
              );
            })}
          </div>
        )}
        <p className="muted">
          Tip: click a category to cross-filter{token ? "; drag tiles to reorder" : ""}.
        </p>

        {error && <p className="app__error">{error}</p>}
        <div className="dash__grid">
          {layout.map((item, i) => {
            const r = resultFor(item.card_id);
            return (
              <div
                className="dash__tile"
                key={item.card_id}
                style={{ gridColumn: item.w === 2 ? "span 2" : "span 1" }}
                draggable={!!token}
                onDragStart={() => setDragIndex(i)}
                onDragOver={(e) => e.preventDefault()}
                onDrop={() => onDrop(i)}
              >
                <div className="dash__tilehead">
                  <h3>{r?.name ?? cardTitle(item.card_id)}</h3>
                  {token && (
                    <button className="link" onClick={() => toggleWidth(i)}>
                      {item.w === 2 ? "½" : "▭"}
                    </button>
                  )}
                </div>
                {r?.result ? (
                  <ResultView result={r.result} onSelect={crossFilter} />
                ) : (
                  <p className="app__error">{r?.error ?? "no result"}</p>
                )}
              </div>
            );
          })}
        </div>
      </div>
    );
  }

  return (
    <div className="dash">
      <h2>Dashboards</h2>
      {dashboards.length === 0 ? (
        <p className="muted">No dashboards yet.</p>
      ) : (
        <ul className="saved__list">
          {dashboards.map((d) => (
            <li key={d.id}>
              <button className="link" onClick={() => openDash(d)}>
                {d.name}
              </button>
              <span className="muted">
                {" "}
                · {d.card_ids.length} card(s)
                {(d.parameters?.length ?? 0) > 0 ? ` · ${d.parameters!.length} filter(s)` : ""}
              </span>
            </li>
          ))}
        </ul>
      )}

      {token && (
        <div className="dash__new">
          <h3>New dashboard</h3>
          <input placeholder="name" value={name} onChange={(e) => setName(e.target.value)} />

          <p className="muted">Cards</p>
          <div className="chips">
            {cards.map((c) => (
              <label key={c.id} className="chip">
                <input
                  type="checkbox"
                  checked={selected.includes(c.id)}
                  onChange={() => toggle(c.id)}
                />
                {c.name}
              </label>
            ))}
          </div>

          <p className="muted">
            Shared filters{" "}
            <button className="link" onClick={addParam}>
              + add
            </button>
          </p>
          {params.map((p, i) => (
            <div className="dash__row" key={i}>
              <input
                placeholder="filter name"
                value={p.name}
                onChange={(e) => setParamField(i, { name: e.target.value })}
              />
              <select
                value={p.kind}
                onChange={(e) => setParamField(i, { kind: e.target.value as ParamKind })}
              >
                <option value="text">text</option>
                <option value="number">number</option>
              </select>
            </div>
          ))}

          {params.length > 0 && (
            <>
              <p className="muted">
                Bindings (filter → card field){" "}
                <button className="link" onClick={addBinding}>
                  + add
                </button>
              </p>
              {bindings.map((b, i) => (
                <div className="dash__row" key={i}>
                  <select
                    value={b.parameter}
                    onChange={(e) => setBindingField(i, { parameter: e.target.value })}
                  >
                    {params.map((p) => (
                      <option key={p.name} value={p.name}>
                        {p.name}
                      </option>
                    ))}
                  </select>
                  <select
                    value={b.card_id}
                    onChange={(e) => setBindingField(i, { card_id: e.target.value })}
                  >
                    {selected.map((id) => (
                      <option key={id} value={id}>
                        {cardTitle(id)}
                      </option>
                    ))}
                  </select>
                  <input
                    placeholder="field"
                    value={b.field}
                    onChange={(e) => setBindingField(i, { field: e.target.value })}
                  />
                </div>
              ))}
            </>
          )}

          {dashboards.length > 0 && (
            <>
              <p className="muted">Linked dashboards</p>
              <div className="chips">
                {dashboards.map((d) => (
                  <label key={d.id} className="chip">
                    <input
                      type="checkbox"
                      checked={links.includes(d.id)}
                      onChange={() => toggleLink(d.id)}
                    />
                    {d.name}
                  </label>
                ))}
              </div>
            </>
          )}

          <div className="builder__actions">
            <button onClick={create} disabled={!name || selected.length === 0}>
              Create dashboard
            </button>
          </div>
        </div>
      )}
      {error && <p className="app__error">{error}</p>}
    </div>
  );
}
