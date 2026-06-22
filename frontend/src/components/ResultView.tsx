import { useState } from "react";
import type { QueryResult } from "../api/client";
import {
  chartData,
  comboData,
  isChartable,
  isComboable,
  isPivotable,
  isScatterable,
  pivot,
  type ChartKind,
} from "../lib/viz";
import {
  NivoBar,
  NivoCombo,
  NivoLine,
  NivoPie,
  NivoScatter,
} from "./charts/NivoCharts";

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
  const scatterable = isScatterable(result);
  const comboable = isComboable(result);
  const pivotable = isPivotable(result);
  const kinds: ChartKind[] = chartable
    ? scatterable
      ? ["table", "scatter", "bar", "line", "area", "funnel", "pie"]
      : ["table", "bar", "line", "area", "funnel", "pie"]
    : comboable
      ? ["table", "combo", "pivot"]
      : pivotable
        ? ["table", "pivot"]
        : ["table"];
  const [kind, setKind] = useState<ChartKind>(
    chartable ? (scatterable ? "scatter" : "bar") : comboable ? "combo" : "table",
  );
  const data = chartable ? chartData(result) : { labels: [], values: [] };
  const active = kinds.includes(kind) ? kind : "table";

  const indexName = columns[0] ?? "";
  const valueName = columns[1] ?? "value";
  const pickValue = onSelect ? (v: string) => onSelect(columns[0], v) : undefined;

  // Funnel = bars sorted by value, descending.
  const funnel = data.labels
    .map((label, i) => ({ label, value: data.values[i] }))
    .sort((a, b) => b.value - a.value);

  // Charts that need the sized rectangular container (everything but pie,
  // which has its own square container, and the table/pivot text views).
  const isRectChart = active !== "table" && active !== "pivot" && active !== "pie";

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

      {/* Sized container: nivo's responsive charts measure their parent. */}
      {isRectChart && (
        <div className="nivo-chart">
          {active === "bar" && (
            <NivoBar
              labels={data.labels}
              values={data.values}
              indexName={indexName}
              valueName={valueName}
              onSelect={pickValue}
            />
          )}
          {active === "line" && (
            <NivoLine labels={data.labels} values={data.values} xName={indexName} yName={valueName} />
          )}
          {active === "area" && (
            <NivoLine
              labels={data.labels}
              values={data.values}
              xName={indexName}
              yName={valueName}
              area
            />
          )}
          {active === "scatter" && <NivoScatter result={result} />}
          {active === "funnel" && (
            <NivoBar
              labels={funnel.map((f) => f.label)}
              values={funnel.map((f) => f.value)}
              indexName={indexName}
              valueName={valueName}
              horizontal
              onSelect={pickValue}
            />
          )}
          {active === "combo" && (
            <NivoCombo
              labels={comboData(result).labels}
              bars={comboData(result).bars}
              line={comboData(result).line}
              barName={columns[1] ?? "bars"}
              lineName={columns[2] ?? "line"}
            />
          )}
        </div>
      )}

      {active === "pie" && (
        <div className="nivo-chart nivo-chart--pie">
          <NivoPie labels={data.labels} values={data.values} onSelect={pickValue} />
        </div>
      )}

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
