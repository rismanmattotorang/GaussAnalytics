import { useState } from "react";
import type { QueryResult } from "../api/client";
import {
  chartData,
  isChartable,
  isPivotable,
  linePoints,
  pieSlices,
  pivot,
  type ChartKind,
} from "../lib/viz";

function PivotTable({ result }: { result: QueryResult }) {
  const p = pivot(result);
  return (
    <div className="table-scroll">
      <table className="data-table">
        <thead>
          <tr>
            <th>{result.columns[0]}</th>
            {p.columns.map((c) => (
              <th key={c}>{c}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {p.rows.map((row) => (
            <tr key={row.label}>
              <td>{row.label}</td>
              {row.cells.map((cell, j) => (
                <td key={j}>{cell === null ? "∅" : String(cell)}</td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

const PALETTE = ["#38bdf8", "#818cf8", "#34d399", "#fbbf24", "#f87171", "#a78bfa", "#f472b6"];

function BarChart({
  labels,
  values,
  onSelect,
}: {
  labels: string[];
  values: number[];
  onSelect?: (value: string) => void;
}) {
  const max = Math.max(1, ...values);
  return (
    <div className="chart">
      {values.map((v, i) => (
        <div className="chart__row" key={i}>
          <span
            className={onSelect ? "chart__label chart__label--click" : "chart__label"}
            onClick={onSelect ? () => onSelect(labels[i]) : undefined}
          >
            {labels[i]}
          </span>
          <span className="chart__bar" style={{ width: `${(v / max) * 100}%` }} />
          <span className="chart__value">{v}</span>
        </div>
      ))}
    </div>
  );
}

function LineChart({ values }: { values: number[] }) {
  const w = 600;
  const h = 160;
  return (
    <svg className="linechart" viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none">
      <polyline points={linePoints(values, w, h)} fill="none" stroke="#38bdf8" strokeWidth="2" />
    </svg>
  );
}

function PieChart({ labels, values }: { labels: string[]; values: number[] }) {
  const slices = pieSlices(values);
  const r = 60;
  const c = 2 * Math.PI * r;
  return (
    <div className="pie">
      <svg viewBox="0 0 160 160" width="160" height="160">
        <g transform="translate(80,80) rotate(-90)">
          {slices.map((s, i) => (
            <circle
              key={i}
              r={r}
              fill="none"
              stroke={PALETTE[i % PALETTE.length]}
              strokeWidth="36"
              strokeDasharray={`${s.frac * c} ${c}`}
              strokeDashoffset={`${-s.start * c}`}
            />
          ))}
        </g>
      </svg>
      <ul className="pie__legend">
        {labels.map((l, i) => (
          <li key={i}>
            <span className="pie__swatch" style={{ background: PALETTE[i % PALETTE.length] }} />
            {l} ({values[i]})
          </li>
        ))}
      </ul>
    </div>
  );
}

export function ResultView({
  result,
  onSelect,
}: {
  result: QueryResult;
  /** Called when the user clicks a first-column category value (cross-filter). */
  onSelect?: (column: string, value: unknown) => void;
}) {
  const { columns, rows } = result;
  const chartable = isChartable(result);
  const pivotable = isPivotable(result);
  const kinds: ChartKind[] = chartable
    ? ["table", "bar", "line", "pie"]
    : pivotable
      ? ["table", "pivot"]
      : ["table"];
  const [kind, setKind] = useState<ChartKind>(chartable ? "bar" : "table");
  const data = chartable ? chartData(result) : { labels: [], values: [] };
  const active = kinds.includes(kind) ? kind : "table";

  return (
    <div className="result">
      <div className="result__head">
        <span className="muted">{rows.length} row(s)</span>
        {kinds.length > 1 && (
          <span className="viz-pick">
            {kinds.map((k) => (
              <button key={k} className="link" data-active={k === active} onClick={() => setKind(k)}>
                {k}
              </button>
            ))}
          </span>
        )}
      </div>

      {active === "bar" && (
        <BarChart
          labels={data.labels}
          values={data.values}
          onSelect={onSelect ? (v) => onSelect(columns[0], v) : undefined}
        />
      )}
      {active === "line" && <LineChart values={data.values} />}
      {active === "pie" && <PieChart labels={data.labels} values={data.values} />}
      {active === "pivot" && <PivotTable result={result} />}

      {active === "table" && (
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
                    <td
                      key={j}
                      className={onSelect && j === 0 ? "td--click" : undefined}
                      onClick={
                        onSelect && j === 0 ? () => onSelect(columns[0], cell) : undefined
                      }
                    >
                      {cell === null ? "∅" : String(cell)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
