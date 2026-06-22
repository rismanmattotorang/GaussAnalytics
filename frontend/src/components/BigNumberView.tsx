import type { QueryResult } from "../api/client";

// A single headline value taken from the first cell of a result/DataFrame, with
// the first column name as its label. Shared by notebook Big Number cells and
// the dashboard notebook-card renderer.
export function BigNumberView({ result }: { result: QueryResult }) {
  const value = result.rows[0]?.[0];
  const label = result.columns[0] ?? "";
  return (
    <div className="big-number">
      <div className="big-number__value">
        {value === null || value === undefined ? "∅" : String(value)}
      </div>
      <div className="big-number__label">{label}</div>
    </div>
  );
}
