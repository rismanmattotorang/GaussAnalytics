import { useState } from "react";
import { api, type Database, type DataSourceKind } from "../api/client";

const KINDS: DataSourceKind[] = [
  "sqlite",
  "postgres",
  "mysql",
  "oracle",
  "snowflake",
  "bigquery",
  "clickhouse",
  "generic",
];

/** A connection-string example per kind, shown as the input placeholder. */
const URI_HINTS: Record<DataSourceKind, string> = {
  sqlite: "sqlite://data/source.db",
  postgres: "postgres://user:pass@host:5432/dbname",
  mysql: "mysql://user:pass@host:3306/dbname",
  oracle: "oracle://host/ords/schema?user=USER&password=PASS",
  snowflake: "snowflake://account?token=…&database=DB&schema=SC&warehouse=WH",
  bigquery: "bigquery://project?dataset=…&token=…",
  clickhouse: "clickhouse://host:8123?database=…",
  generic: "(standard-SQL endpoint)",
};

export function DataSources({
  databases,
  token,
  onChange,
}: {
  databases: Database[];
  token: string | null;
  /** Reload the source list after a mutation. */
  onChange: () => void;
}) {
  const [name, setName] = useState("");
  const [kind, setKind] = useState<DataSourceKind>("sqlite");
  const [uri, setUri] = useState("");
  const [busy, setBusy] = useState(false);
  const [test, setTest] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  if (!token) {
    return <p className="muted">Sign in as an administrator to manage data sources.</p>;
  }

  function fail(e: unknown) {
    setError(e instanceof Error ? e.message : String(e));
  }

  async function testConnection() {
    setError(null);
    setTest(null);
    setBusy(true);
    try {
      const r = await api.testConnection({ kind, connection_uri: uri }, token!);
      setTest(`Connection OK — discovered ${r.table_count} table(s).`);
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }

  async function add() {
    setError(null);
    setBusy(true);
    try {
      await api.createDatabase({ name, kind, connection_uri: uri || undefined }, token!);
      setName("");
      setUri("");
      setTest(null);
      onChange();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }

  async function sync(id: string) {
    setError(null);
    setBusy(true);
    try {
      await api.syncDatabase(id, token!);
      onChange();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }

  async function remove(id: string, dbName: string) {
    if (!confirm(`Delete data source "${dbName}"? Its synced tables are removed too.`)) return;
    setError(null);
    setBusy(true);
    try {
      await api.deleteDatabase(id, token!);
      onChange();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="datasources">
      {error && <p className="app__error">{error}</p>}

      <h3>Connected data sources</h3>
      {databases.length === 0 ? (
        <p className="muted">No data sources yet. Add one below.</p>
      ) : (
        <table className="data-table">
          <thead>
            <tr>
              <th>Name</th>
              <th>Kind</th>
              <th>Synced</th>
              <th>Actions</th>
            </tr>
          </thead>
          <tbody>
            {databases.map((d) => (
              <tr key={d.id}>
                <td>{d.name}</td>
                <td>{d.kind}</td>
                <td>{d.is_synced ? "yes" : "no"}</td>
                <td>
                  <button className="link" disabled={busy} onClick={() => sync(d.id)}>
                    sync
                  </button>
                  <button className="link" disabled={busy} onClick={() => remove(d.id, d.name)}>
                    delete
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      <h3>Add a data source</h3>
      <div className="ds-form">
        <label>
          Name
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Production warehouse" />
        </label>
        <label>
          Kind
          <select value={kind} onChange={(e) => setKind(e.target.value as DataSourceKind)}>
            {KINDS.map((k) => (
              <option key={k} value={k}>
                {k}
              </option>
            ))}
          </select>
        </label>
        <label className="ds-form__uri">
          Connection URI
          <input value={uri} onChange={(e) => setUri(e.target.value)} placeholder={URI_HINTS[kind]} />
        </label>
        <div className="ds-form__actions">
          <button disabled={busy || !uri} onClick={testConnection}>
            Test
          </button>
          <button disabled={busy || !name} onClick={add}>
            Add
          </button>
        </div>
        {test && <p className="ds-ok">{test}</p>}
      </div>
    </div>
  );
}
