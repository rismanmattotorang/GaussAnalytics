import type { QueryResult } from "../api/client";

function BarChart({ labels, values }: { labels: string[]; values: number[] }) {
  const max = Math.max(1, ...values);
  return (
    <div className="chart">
      {values.map((v, i) => (
        <div className="chart__row" key={i}>
          <span className="chart__label">{labels[i]}</span>
          <span className="chart__bar" style={{ width: `${(v / max) * 100}%` }} />
          <span className="chart__value">{v}</span>
        </div>
      ))}
    </div>
  );
}

export function ResultView({ result }: { result: QueryResult }) {
  const { columns, rows } = result;
  const charty =
    columns.length === 2 &&
    rows.length > 0 &&
    rows.every((r) => typeof r[1] === "number");

  return (
    <div className="result">
      <p className="muted">{rows.length} row(s)</p>
      {charty && (
        <BarChart
          labels={rows.map((r) => String(r[0]))}
          values={rows.map((r) => Number(r[1]))}
        />
      )}
      <div className="table-scroll">
        <table className="data-table">
          <thead>
            <tr>
              {columns.map((c) => (
                <th key={c}>{c}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.slice(0, 200).map((row, i) => (
              <tr key={i}>
                {row.map((cell, j) => (
                  <td key={j}>{cell === null ? "∅" : String(cell)}</td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
