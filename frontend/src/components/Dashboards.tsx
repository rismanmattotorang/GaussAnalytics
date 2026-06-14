import { useEffect, useState } from "react";
import { api, type Card, type Dashboard, type QueryResult } from "../api/client";
import { ResultView } from "./ResultView";

export function Dashboards({ token }: { token: string | null }) {
  const [dashboards, setDashboards] = useState<Dashboard[]>([]);
  const [cards, setCards] = useState<Card[]>([]);
  const [open, setOpen] = useState<Dashboard | null>(null);
  const [results, setResults] = useState<Record<string, QueryResult>>({});
  const [name, setName] = useState("");
  const [selected, setSelected] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);

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

  async function openDash(d: Dashboard) {
    setOpen(d);
    setResults({});
    for (const id of d.card_ids) {
      try {
        const r = await api.runCard(id);
        setResults((prev) => ({ ...prev, [id]: r }));
      } catch {
        /* a single card failing should not break the board */
      }
    }
  }

  function toggle(id: string) {
    setSelected((s) => (s.includes(id) ? s.filter((x) => x !== id) : [...s, id]));
  }

  async function create() {
    if (!token || !name) return;
    setError(null);
    try {
      await api.createDashboard({ name, card_ids: selected }, token);
      setName("");
      setSelected([]);
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
        <div className="dash__grid">
          {open.card_ids.map((id) => (
            <div className="dash__tile" key={id}>
              <h3>{cardTitle(id)}</h3>
              {results[id] ? <ResultView result={results[id]} /> : <p className="muted">running…</p>}
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
              <span className="muted"> · {d.card_ids.length} card(s)</span>
            </li>
          ))}
        </ul>
      )}

      {token && (
        <div className="dash__new">
          <h3>New dashboard</h3>
          <input placeholder="name" value={name} onChange={(e) => setName(e.target.value)} />
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
          <button onClick={create} disabled={!name || selected.length === 0}>
            Create
          </button>
        </div>
      )}
      {error && <p className="app__error">{error}</p>}
    </div>
  );
}
