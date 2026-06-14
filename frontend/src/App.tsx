import { useEffect, useState } from "react";
import { api, type Database, type Health } from "./api/client";

export default function App() {
  const [health, setHealth] = useState<Health | null>(null);
  const [databases, setDatabases] = useState<Database[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([api.health(), api.databases()])
      .then(([h, dbs]) => {
        setHealth(h);
        setDatabases(dbs);
      })
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  }, []);

  return (
    <main className="app">
      <header className="app__header">
        <h1 className="app__brand">GaussAnalytics</h1>
        <p className="app__tagline">
          Fast, secure, AI-native BI — by Gaussian Technologies
        </p>
        {health && (
          <span className="app__status" data-status={health.status}>
            {health.status} · v{health.version}
          </span>
        )}
      </header>

      {error && <p className="app__error">Could not reach the API: {error}</p>}

      <section className="app__panel">
        <h2>Connected data sources</h2>
        {databases.length === 0 ? (
          <p className="muted">No databases yet.</p>
        ) : (
          <ul className="db-list">
            {databases.map((db) => (
              <li key={db.id} className="db-list__item">
                <span className="db-list__name">{db.name}</span>
                <span className="db-list__kind">{db.kind}</span>
                <span className="db-list__sync">
                  {db.is_synced ? "synced" : "not synced"}
                </span>
              </li>
            ))}
          </ul>
        )}
      </section>

      <footer className="app__footer muted">
        The query builder, visualizations, and dashboards build on the typed
        API client in <code>src/api/client.ts</code>.
      </footer>
    </main>
  );
}
