import { useState } from "react";
import { api, type Database, type QueryResult } from "../api/client";
import { ResultView } from "./ResultView";

const SAMPLE = "SELECT 1 AS one";

export function NativeSql({ databases }: { databases: Database[] }) {
  const [databaseId, setDatabaseId] = useState<string>(databases[0]?.id ?? "");
  const [sql, setSql] = useState<string>(SAMPLE);
  const [result, setResult] = useState<QueryResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function run() {
    setError(null);
    try {
      setResult(await api.native(databaseId, sql));
    } catch (e: unknown) {
      setResult(null);
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <div className="native">
      <h2>SQL editor</h2>
      <p className="muted">
        Read-only by construction — only a single <code>SELECT</code>/<code>WITH</code> runs;
        writes and DDL are rejected before reaching the database, and results are cached.
      </p>
      <div className="native__row">
        <select value={databaseId} onChange={(e) => setDatabaseId(e.target.value)}>
          {databases.map((d) => (
            <option key={d.id} value={d.id}>
              {d.name} ({d.kind})
            </option>
          ))}
        </select>
        <button onClick={run} disabled={!databaseId || !sql.trim()}>
          Run
        </button>
      </div>
      <textarea
        className="native__editor"
        value={sql}
        spellCheck={false}
        onChange={(e) => setSql(e.target.value)}
        rows={8}
      />
      {error && <p className="app__error">{error}</p>}
      {result && <ResultView result={result} />}
    </div>
  );
}
