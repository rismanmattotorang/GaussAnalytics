// Pure visualization helpers — no DOM, so they are unit-testable in isolation.

import type { QueryResult } from "../api/client";

export type ChartKind = "table" | "bar" | "line" | "pie";

/** A 2-column result whose second column is numeric can be charted. */
export function isChartable(r: QueryResult): boolean {
  return (
    r.columns.length === 2 &&
    r.rows.length > 0 &&
    r.rows.every((row) => typeof row[1] === "number")
  );
}

export interface ChartData {
  labels: string[];
  values: number[];
}

export function chartData(r: QueryResult): ChartData {
  return {
    labels: r.rows.map((row) => String(row[0])),
    values: r.rows.map((row) => Number(row[1])),
  };
}

/** Cumulative pie slices as `[startFraction, fraction]` pairs (clamped ≥ 0). */
export function pieSlices(values: number[]): Array<{ start: number; frac: number }> {
  const total = values.reduce((a, b) => a + Math.max(0, b), 0) || 1;
  let acc = 0;
  return values.map((v) => {
    const frac = Math.max(0, v) / total;
    const slice = { start: acc, frac };
    acc += frac;
    return slice;
  });
}

/** Points for an SVG polyline over `viewWidth × viewHeight`. */
export function linePoints(values: number[], width: number, height: number): string {
  if (values.length === 0) return "";
  const max = Math.max(1, ...values);
  const step = values.length > 1 ? width / (values.length - 1) : 0;
  return values
    .map((v, i) => `${(i * step).toFixed(1)},${(height - (v / max) * height).toFixed(1)}`)
    .join(" ");
}
