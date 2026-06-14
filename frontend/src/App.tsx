import { useEffect, useState } from "react";
import { api, type Database, type Health } from "./api/client";
import { QueryBuilder } from "./components/QueryBuilder";
import { SavedQuestions } from "./components/SavedQuestions";
import { Dashboards } from "./components/Dashboards";
import { NlAsk } from "./components/NlAsk";

type View = "explore" | "saved" | "dashboards" | "ask";

export default function App() {
  const [health, setHealth] = useState<Health | null>(null);
  const [databases, setDatabases] = useState<Database[]>([]);
  const [view, setView] = useState<View>("explore");
  const [token, setToken] = useState<string | null>(null);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);

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
        {(["explore", "saved", "dashboards", "ask"] as View[]).map((v) => (
          <button key={v} className="tab" data-active={v === view} onClick={() => setView(v)}>
            {v === "explore"
              ? "Explore"
              : v === "saved"
                ? "Saved questions"
                : v === "dashboards"
                  ? "Dashboards"
                  : "Ask (NL2SQL)"}
          </button>
        ))}
      </nav>

      {error && <p className="app__error">{error}</p>}

      <section className="app__panel">
        {databases.length === 0 ? (
          <p className="muted">No data sources, or the API is unreachable.</p>
        ) : view === "explore" ? (
          <QueryBuilder databases={databases} token={token} />
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
