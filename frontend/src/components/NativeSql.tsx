import { useEffect, useMemo, useState } from "react";
import { api, type Database, type QueryResult, type Table } from "../api/client";
import { extractVars, substituteVars } from "../lib/sql";
import { ResultView } from "./ResultView";

const SAMPLE = "SELECT 1 AS one";

const SNIPPETS: Array<[string, string]> = [
  ["select *", "SELECT * FROM  LIMIT 100"],
  ["count", "SELECT count(*) FROM "],
  ["group by", " GROUP BY "],
  ["where {{var}}", " WHERE col = {{value}}"],
];

export function NativeSql({ databases }: { databases: Database[] }) {
  const [databaseId, setDatabaseId] = useState<string>(databases[0]?.id ?? "");
  const [sql, setSql] = useState<string>(SAMPLE);
  const [values, setValues] = useState<Record<string, string>>({});
  const [tables, setTables] = useState<Table[]>([]);
  const [result, setResult] = useState<QueryResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const vars = useMemo(() => extractVars(sql), [sql]);

  useEffect(() => {
    if (!databaseId) return;
    api
      .tables(databaseId)
      .then(setTables)
      .catch(() => setTables([]));
  }, [databaseId]);

  const insert = (text: string) => setSql((s) => s + text);

  async function run() {
    setError(null);
    try {
      const { sql: bound, params } = substituteVars(sql, values);
      setResult(await api.native(databaseId, bound, params));
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
        Use <code>{"{{variable}}"}</code> for bound parameters.
      </p>
      <div className="native__row">
        <select value={databaseId} onChange={(e) => setDatabaseId(e.target.value)}>
          {databases.map((d) => (
            <option key={d.id} value={d.id}>
              {d.name} ({d.kind})
            </option>
          ))}
        </select>
        <select
          value=""
          onChange={(e) => {
            const i = Number(e.target.value);
            if (!Number.isNaN(i) && SNIPPETS[i]) insert(SNIPPETS[i][1]);
            e.target.value = "";
          }}
        >
          <option value="">+ snippet…</option>
          {SNIPPETS.map(([label], i) => (
            <option key={i} value={i}>
              {label}
            </option>
          ))}
        </select>
        <button onClick={run} disabled={!databaseId || !sql.trim()}>
          Run
        </button>
      </div>

      <div className="native__split">
        <textarea
          className="native__editor"
          value={sql}
          spellCheck={false}
          onChange={(e) => setSql(e.target.value)}
          rows={10}
        />
        <div className="native__schema">
          <strong>Schema</strong>
          {tables.length === 0 ? (
            <p className="muted">No synced tables.</p>
          ) : (
            <ul>
              {tables.map((t) => (
                <li key={t.id}>
                  <button className="link" onClick={() => insert(t.name)}>
                    {t.name}
                  </button>
                  <ul>
                    {t.fields.map((f) => (
                      <li key={f.id}>
                        <button className="link" onClick={() => insert(f.name)}>
                          {f.name}
                        </button>{" "}
                        <span className="chip__type">{f.semantic_type ?? f.field_type}</span>
                      </li>
                    ))}
                  </ul>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>

      {vars.length > 0 && (
        <div className="native__vars">
          {vars.map((name) => (
            <label key={name}>
              {name}
              <input
                value={values[name] ?? ""}
                onChange={(e) => setValues((v) => ({ ...v, [name]: e.target.value }))}
              />
            </label>
          ))}
        </div>
      )}
      {error && <p className="app__error">{error}</p>}
      {result && <ResultView result={result} />}
    </div>
  );
}
