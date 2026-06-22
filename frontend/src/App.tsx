import { useEffect, useState } from "react";
import { api, type Database, type Health } from "./api/client";
import { QueryBuilder } from "./components/QueryBuilder";
import { SavedQuestions } from "./components/SavedQuestions";
import { Dashboards } from "./components/Dashboards";
import { Notebooks } from "./components/Notebooks";
import { NativeSql } from "./components/NativeSql";
import { NlAsk } from "./components/NlAsk";
import { DataSources } from "./components/DataSources";
import { Settings } from "./components/Settings";

type View =
  | "explore"
  | "sql"
  | "saved"
  | "dashboards"
  | "notebooks"
  | "ask"
  | "data"
  | "settings";

const VIEW_LABELS: Record<View, string> = {
  explore: "Explore",
  sql: "SQL",
  saved: "Saved questions",
  dashboards: "Dashboards",
  notebooks: "Notebooks",
  ask: "Ask (NL2SQL)",
  data: "Data sources",
  settings: "Settings",
};

export default function App() {
  const [health, setHealth] = useState<Health | null>(null);
  const [databases, setDatabases] = useState<Database[]>([]);
  const [view, setView] = useState<View>("explore");
  const [token, setToken] = useState<string | null>(null);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);

  function reloadDatabases() {
    api
      .databases()
      .then(setDatabases)
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  }

  useEffect(() => {
    Promise.all([api.health(), api.databases()])
      .then(([h, dbs]) => {
        setHealth(h);
        setDatabases(dbs);
      })
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  }, []);

  async function login() {
    setError(null);
    try {
      const session = await api.login(email, password);
      setToken(session.token);
      setPassword("");
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <main className="app">
      <header className="app__header">
        <div>
          <h1 className="app__brand">GaussAnalytics</h1>
          <p className="app__tagline">Fast, secure, AI-native BI — by Gaussian Technologies</p>
        </div>
        <div className="app__session">
          {health && (
            <span className="app__status" data-status={health.status}>
              {health.status} · v{health.version}
            </span>
          )}
          {token ? (
            <button className="link" onClick={() => setToken(null)}>
              sign out
            </button>
          ) : (
            <span className="login">
              <input placeholder="email" value={email} onChange={(e) => setEmail(e.target.value)} />
              <input
                placeholder="password"
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
              />
              <button onClick={login} disabled={!email || !password}>
                sign in
              </button>
            </span>
          )}
        </div>
      </header>

      <nav className="app__nav">
        {(
          [
            "explore",
            "sql",
            "saved",
            "dashboards",
            "notebooks",
            "ask",
            "data",
            "settings",
          ] as View[]
        ).map((v) => (
          <button key={v} className="tab" data-active={v === view} onClick={() => setView(v)}>
            {VIEW_LABELS[v]}
          </button>
        ))}
      </nav>

      {error && <p className="app__error">{error}</p>}

      <section className="app__panel">
        {view === "data" ? (
          // Admin: manage data sources — must work even with none connected yet.
          <DataSources databases={databases} token={token} onChange={reloadDatabases} />
        ) : view === "settings" ? (
          <Settings token={token} />
        ) : view === "notebooks" ? (
          // Notebooks don't require a data source (Python cells run on Jupyter);
          // SQL/NL2SQL cells use the connected sources when present.
          <Notebooks token={token} databases={databases} />
        ) : databases.length === 0 ? (
          <p className="muted">
            No data sources yet — add one in the <strong>Data sources</strong> tab.
          </p>
        ) : view === "explore" ? (
          <QueryBuilder databases={databases} token={token} />
        ) : view === "sql" ? (
          <NativeSql databases={databases} />
        ) : view === "saved" ? (
          <SavedQuestions />
        ) : view === "dashboards" ? (
          <Dashboards token={token} />
        ) : (
          <NlAsk databases={databases} />
        )}
      </section>

      <footer className="app__footer muted">
        Built on the typed API client in <code>src/api/client.ts</code> — the same
        contract the Rust server and admin TUI speak.
      </footer>
    </main>
  );
}
