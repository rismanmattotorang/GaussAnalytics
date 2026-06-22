// Nivo-backed chart components for GaussAnalytics.
//
// We adopt nivo (https://nivo.rocks — a D3-powered React chart library) for the
// web UI's result and dashboard charts: responsive layout, rich tooltips,
// legends, axis titles, and animation, with a single shared dark theme tuned to
// the app palette. Each wrapper takes the plain `{labels, values}` shapes from
// `lib/viz.ts`, so the rest of the app stays decoupled from nivo's data model.

import { ResponsiveBar } from "@nivo/bar";
import { ResponsiveLine } from "@nivo/line";
import { ResponsivePie } from "@nivo/pie";
import { ResponsiveScatterPlot } from "@nivo/scatterplot";
import type { QueryResult } from "../../api/client";

export const PALETTE = [
  "#38bdf8",
  "#818cf8",
  "#34d399",
  "#fbbf24",
  "#f87171",
  "#a78bfa",
  "#f472b6",
];

// Shared theme: legible on the app's dark surfaces.
const THEME = {
  background: "transparent",
  text: { fill: "#cbd5e1", fontSize: 11 },
  axis: {
    ticks: { text: { fill: "#94a3b8", fontSize: 10 } },
    legend: { text: { fill: "#cbd5e1", fontSize: 11 } },
  },
  grid: { line: { stroke: "#1e293b", strokeWidth: 1 } },
  legends: { text: { fill: "#cbd5e1" } },
  tooltip: {
    container: {
      background: "#0f172a",
      color: "#e2e8f0",
      fontSize: 12,
      border: "1px solid #334155",
      borderRadius: 6,
    },
  },
};

const MARGIN = { top: 16, right: 24, bottom: 56, left: 64 };

/** Rotate x tick labels once there are enough categories to overlap. */
function tickRotation(n: number): number {
  return n > 6 ? -35 : 0;
}

export function NivoBar({
  labels,
  values,
  indexName,
  valueName,
  horizontal,
  onSelect,
}: {
  labels: string[];
  values: number[];
  indexName: string;
  valueName: string;
  horizontal?: boolean;
  onSelect?: (value: string) => void;
}) {
  const data = labels.map((label, i) => ({ label, value: values[i] }));
  return (
    <ResponsiveBar
      data={data}
      keys={["value"]}
      indexBy="label"
      layout={horizontal ? "horizontal" : "vertical"}
      margin={MARGIN}
      padding={0.3}
      colors={PALETTE[0]}
      theme={THEME}
      animate
      enableLabel={false}
      axisBottom={{
        tickRotation: horizontal ? 0 : tickRotation(labels.length),
        legend: horizontal ? valueName : indexName,
        legendOffset: 44,
        legendPosition: "middle",
      }}
      axisLeft={{
        legend: horizontal ? indexName : valueName,
        legendOffset: -52,
        legendPosition: "middle",
      }}
      onClick={onSelect ? (d) => onSelect(String(d.indexValue)) : undefined}
      role="img"
      ariaLabel={`${valueName} by ${indexName}`}
    />
  );
}

export function NivoLine({
  labels,
  values,
  xName,
  yName,
  area,
}: {
  labels: string[];
  values: number[];
  xName: string;
  yName: string;
  area?: boolean;
}) {
  const data = [
    { id: yName || "series", data: labels.map((x, i) => ({ x, y: values[i] })) },
  ];
  return (
    <ResponsiveLine
      data={data}
      margin={MARGIN}
      theme={THEME}
      colors={PALETTE}
      curve="monotoneX"
      enableArea={!!area}
      areaOpacity={0.18}
      pointSize={6}
      pointBorderWidth={1}
      useMesh
      animate
      axisBottom={{
        tickRotation: tickRotation(labels.length),
        legend: xName,
        legendOffset: 44,
        legendPosition: "middle",
      }}
      axisLeft={{ legend: yName, legendOffset: -52, legendPosition: "middle" }}
    />
  );
}

export function NivoPie({
  labels,
  values,
  onSelect,
}: {
  labels: string[];
  values: number[];
  onSelect?: (value: string) => void;
}) {
  const data = labels.map((label, i) => ({
    id: label,
    label,
    value: Math.max(0, values[i]),
  }));
  return (
    <ResponsivePie
      data={data}
      margin={{ top: 24, right: 24, bottom: 56, left: 24 }}
      theme={THEME}
      colors={PALETTE}
      innerRadius={0.5}
      padAngle={1}
      cornerRadius={3}
      activeOuterRadiusOffset={8}
      borderWidth={1}
      borderColor={{ from: "color", modifiers: [["darker", 0.3]] }}
      arcLinkLabelsColor={{ from: "color" }}
      arcLinkLabelsThickness={2}
      arcLabelsSkipAngle={14}
      arcLabelsTextColor="#0b1220"
      animate
      legends={[
        {
          anchor: "bottom",
          direction: "row",
          translateY: 48,
          itemWidth: 84,
          itemHeight: 16,
          symbolSize: 12,
          itemTextColor: "#cbd5e1",
        },
      ]}
      onClick={onSelect ? (d) => onSelect(String(d.id)) : undefined}
    />
  );
}

export function NivoScatter({ result }: { result: QueryResult }) {
  const data = [
    {
      id: result.columns[1] ?? "y",
      data: result.rows.map((row) => ({ x: Number(row[0]), y: Number(row[1]) })),
    },
  ];
  return (
    <ResponsiveScatterPlot
      data={data}
      margin={MARGIN}
      theme={THEME}
      colors={PALETTE}
      nodeSize={8}
      useMesh
      animate
      axisBottom={{
        legend: result.columns[0] ?? "x",
        legendOffset: 44,
        legendPosition: "middle",
      }}
      axisLeft={{
        legend: result.columns[1] ?? "y",
        legendOffset: -52,
        legendPosition: "middle",
      }}
    />
  );
}

/**
 * Combo chart: bars for the first measure with the second measure overlaid as a
 * line, sharing one Y scale. The line is a custom nivo layer drawn from the bar
 * nodes' positions so the two series register exactly.
 */
export function NivoCombo({
  labels,
  bars,
  line,
  barName,
  lineName,
}: {
  labels: string[];
  bars: number[];
  line: number[];
  barName: string;
  lineName: string;
}) {
  const data = labels.map((label, i) => ({ label, [barName]: bars[i] }));
  const maxValue = Math.max(1, ...bars, ...line);

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const LineLayer = ({ bars: barNodes, yScale }: any) => {
    if (!barNodes?.length) return null;
    const pts = barNodes.map(
      (b: { x: number; width: number }, i: number) =>
        `${b.x + b.width / 2},${(yScale as (v: number) => number)(line[i] ?? 0)}`,
    );
    return (
      <g>
        <polyline points={pts.join(" ")} fill="none" stroke="#fbbf24" strokeWidth={2} />
        {pts.map((p: string, i: number) => {
          const [cx, cy] = p.split(",");
          return <circle key={i} cx={cx} cy={cy} r={3} fill="#fbbf24" />;
        })}
      </g>
    );
  };

  return (
    <ResponsiveBar
      data={data}
      keys={[barName]}
      indexBy="label"
      margin={MARGIN}
      padding={0.3}
      valueScale={{ type: "linear", min: 0, max: maxValue }}
      colors={PALETTE[0]}
      theme={THEME}
      enableLabel={false}
      animate
      axisBottom={{
        tickRotation: tickRotation(labels.length),
        legend: `${barName} (bars) · ${lineName} (line)`,
        legendOffset: 44,
        legendPosition: "middle",
      }}
      axisLeft={{ legendOffset: -52, legendPosition: "middle" }}
      layers={["grid", "axes", "bars", LineLayer, "markers", "legends"]}
      role="img"
    />
  );
}
