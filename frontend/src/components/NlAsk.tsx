import { useState } from "react";
import { api, type Database, type GuardedQuery } from "../api/client";

export function NlAsk({ databases }: { databases: Database[] }) {
  const [databaseId, setDatabaseId] = useState<string>(databases[0]?.id ?? "");
  const [prompt, setPrompt] = useState<string>("");
  const [answer, setAnswer] = useState<GuardedQuery | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function ask() {
    setError(null);
    setAnswer(null);
    try {
      setAnswer(await api.nl2sql(databaseId, prompt));
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <div className="nlask">
      <h2>Ask in natural language</h2>
      <p className="muted">
        Powered by Gaussian NL2SQL — output is schema-grounded, read-only-guardrailed,
        and runs under your permissions. (Enable with <code>GAUSS_NL2SQL_ENABLED=true</code>.)
      </p>
      <div className="nlask__row">
        <select value={databaseId} onChange={(e) => setDatabaseId(e.target.value)}>
          {databases.map((d) => (
            <option key={d.id} value={d.id}>
              {d.name}
            </option>
          ))}
        </select>
        <input
          placeholder="e.g. total revenue by status last month"
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
        />
        <button onClick={ask} disabled={!prompt}>
          Ask
        </button>
      </div>
      {answer && (
        <pre className="sql">
          {answer.sql}
          {answer.explanation ? `\n-- ${answer.explanation}` : ""}
        </pre>
      )}
      {error && <p className="app__error">{error}</p>}
    </div>
  );
}
