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
import { ResultView } from "./ResultView";

export function Dashboards({ token }: { token: string | null }) {
  const [dashboards, setDashboards] = useState<Dashboard[]>([]);
  const [cards, setCards] = useState<Card[]>([]);
  const [open, setOpen] = useState<Dashboard | null>(null);
  const [results, setResults] = useState<DashboardCardResult[]>([]);
  const [filterValues, setFilterValues] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);

  // Create-form state.
  const [name, setName] = useState("");
  const [selected, setSelected] = useState<string[]>([]);
  const [params, setParams] = useState<DashboardParameter[]>([]);
  const [bindings, setBindings] = useState<ParamBinding[]>([]);

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
    await runBoard(d, {});
  }

  // --- create-form helpers ---
  function toggle(id: string) {
    setSelected((s) => (s.includes(id) ? s.filter((x) => x !== id) : [...s, id]));
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
        { name, card_ids: selected, parameters: params, bindings },
        token,
      );
      setName("");
      setSelected([]);
      setParams([]);
      setBindings([]);
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

        {(open.parameters?.length ?? 0) > 0 && (
          <div className="dash__filters">
            {open.parameters!.map((p) => (
              <label key={p.name}>
                {p.name}
                <input
                  type={p.kind === "number" ? "number" : "text"}
                  value={filterValues[p.name] ?? ""}
                  onChange={(e) =>
                    setFilterValues((v) => ({ ...v, [p.name]: e.target.value }))
                  }
                />
              </label>
            ))}
            <button
              onClick={() => {
                const values: Record<string, unknown> = {};
                for (const [k, v] of Object.entries(filterValues)) {
                  if (v !== "") values[k] = v;
                }
                runBoard(open, values);
              }}
            >
              Apply filters
            </button>
          </div>
        )}

        {error && <p className="app__error">{error}</p>}
        <div className="dash__grid">
          {results.map((r) => (
            <div className="dash__tile" key={r.card_id}>
              <h3>{r.name}</h3>
              {r.result ? (
                <ResultView result={r.result} />
              ) : (
                <p className="app__error">{r.error ?? "no result"}</p>
              )}
            </div>
          ))}
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
                {(d.parameters?.length ?? 0) > 0
                  ? ` · ${d.parameters!.length} filter(s)`
                  : ""}
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
            Shared filters <button className="link" onClick={addParam}>+ add</button>
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
