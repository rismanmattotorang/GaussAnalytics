import { useEffect, useState } from "react";
import {
  api,
  type AggFunc,
  type Database,
  type Query,
  type QueryResult,
  type Table,
} from "../api/client";
import { ResultView } from "./ResultView";

const AGG_FUNCS: AggFunc[] = ["count", "sum", "avg", "min", "max", "count_distinct"];

export function QueryBuilder({ databases, token }: { databases: Database[]; token: string | null }) {
  const [databaseId, setDatabaseId] = useState<string>(databases[0]?.id ?? "");
  const [tables, setTables] = useState<Table[]>([]);
  const [tableName, setTableName] = useState<string>("");
  const [fields, setFields] = useState<string[]>([]);
  const [aggFunc, setAggFunc] = useState<AggFunc | "">("");
  const [aggField, setAggField] = useState<string>("");
  const [breakout, setBreakout] = useState<string>("");
  const [limit, setLimit] = useState<string>("100");
  const [result, setResult] = useState<QueryResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [cardName, setCardName] = useState<string>("");
  const [saved, setSaved] = useState<string | null>(null);

  useEffect(() => {
    if (!databaseId) return;
    api
      .tables(databaseId)
      .then((t) => {
        setTables(t);
        setTableName(t[0]?.name ?? "");
      })
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  }, [databaseId]);

  const currentTable = tables.find((t) => t.name === tableName);

  function toggleField(name: string) {
    setFields((f) => (f.includes(name) ? f.filter((x) => x !== name) : [...f, name]));
  }

  function buildQuery(): Query {
    const q: Query = { source_table: tableName };
    if (aggFunc) {
      q.aggregations = [
        { func: aggFunc, field: aggField || null, alias: "value" },
      ];
      if (breakout) q.breakouts = [breakout];
    } else {
      q.fields = fields;
    }
    const n = parseInt(limit, 10);
    if (!Number.isNaN(n)) q.limit = n;
    return q;
  }

  async function run() {
    setError(null);
    setSaved(null);
    try {
      setResult(await api.run(databaseId, buildQuery()));
    } catch (e: unknown) {
      setResult(null);
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  async function save() {
    if (!token || !cardName) return;
    setError(null);
    try {
      await api.createCard({ name: cardName, database_id: databaseId, query: buildQuery() }, token);
      setSaved(`Saved "${cardName}"`);
      setCardName("");
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <div className="builder">
      <div className="builder__controls">
        <label>
          Database
          <select value={databaseId} onChange={(e) => setDatabaseId(e.target.value)}>
            {databases.map((d) => (
              <option key={d.id} value={d.id}>
                {d.name} ({d.kind})
              </option>
            ))}
          </select>
        </label>
        <label>
          Table
          <select value={tableName} onChange={(e) => setTableName(e.target.value)}>
            {tables.map((t) => (
              <option key={t.id} value={t.name}>
                {t.name}
              </option>
            ))}
          </select>
        </label>
        <label>
          Limit
          <input value={limit} onChange={(e) => setLimit(e.target.value)} style={{ width: "5rem" }} />
        </label>
      </div>

      {currentTable && (
        <div className="builder__fields">
          <strong>Fields</strong>
          <div className="chips">
            {currentTable.fields.map((f) => (
              <label key={f.id} className="chip">
                <input
                  type="checkbox"
                  checked={fields.includes(f.name)}
                  onChange={() => toggleField(f.name)}
                />
                {f.name}
                <span className="chip__type">{f.semantic_type ?? f.field_type}</span>
              </label>
            ))}
          </div>
        </div>
      )}

      <div className="builder__agg">
        <label>
          Summarize
          <select value={aggFunc} onChange={(e) => setAggFunc(e.target.value as AggFunc | "")}>
            <option value="">— none (raw rows) —</option>
            {AGG_FUNCS.map((a) => (
              <option key={a} value={a}>
                {a}
              </option>
            ))}
          </select>
        </label>
        {aggFunc && aggFunc !== "count" && currentTable && (
          <label>
            of
            <select value={aggField} onChange={(e) => setAggField(e.target.value)}>
              <option value="">—</option>
              {currentTable.fields.map((f) => (
                <option key={f.id} value={f.name}>
                  {f.name}
                </option>
              ))}
            </select>
          </label>
        )}
        {aggFunc && currentTable && (
          <label>
            grouped by
            <select value={breakout} onChange={(e) => setBreakout(e.target.value)}>
              <option value="">—</option>
              {currentTable.fields.map((f) => (
                <option key={f.id} value={f.name}>
                  {f.name}
                </option>
              ))}
            </select>
          </label>
        )}
      </div>

      <div className="builder__actions">
        <button onClick={run} disabled={!tableName}>
          Run
        </button>
        {token && (
          <>
            <input
              placeholder="Save as…"
              value={cardName}
              onChange={(e) => setCardName(e.target.value)}
            />
            <button onClick={save} disabled={!cardName || !tableName}>
              Save question
            </button>
          </>
        )}
      </div>

      {saved && <p className="ok">{saved}</p>}
      {error && <p className="app__error">{error}</p>}
      {result && <ResultView result={result} />}
    </div>
  );
}
