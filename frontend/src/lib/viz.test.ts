import { describe, expect, it } from "vitest";
import {
  chartData,
  isChartable,
  isPivotable,
  isScatterable,
  linePoints,
  pieSlices,
  pivot,
  scatterPoints,
} from "./viz";

describe("viz helpers", () => {
  it("detects chartable (2 cols, numeric second)", () => {
    expect(isChartable({ columns: ["a", "b"], rows: [["x", 1]] })).toBe(true);
    expect(isChartable({ columns: ["a", "b", "c"], rows: [["x", 1, 2]] })).toBe(false);
    expect(isChartable({ columns: ["a", "b"], rows: [["x", "y"]] })).toBe(false);
    expect(isChartable({ columns: ["a", "b"], rows: [] })).toBe(false);
  });

  it("extracts labels and values", () => {
    const d = chartData({ columns: ["a", "b"], rows: [["x", 1], ["y", 2]] });
    expect(d.labels).toEqual(["x", "y"]);
    expect(d.values).toEqual([1, 2]);
  });

  it("pie slices are cumulative and sum to 1", () => {
    const slices = pieSlices([1, 1, 2]);
    expect(slices[0].frac).toBeCloseTo(0.25);
    const last = slices[slices.length - 1];
    expect(last.start + last.frac).toBeCloseTo(1);
  });

  it("line points has one point per value", () => {
    expect(linePoints([1, 2, 3], 100, 100).split(" ")).toHaveLength(3);
    expect(linePoints([], 100, 100)).toBe("");
  });

  it("detects and scales scatter points", () => {
    const r = { columns: ["x", "y"], rows: [[0, 0], [10, 100]] };
    expect(isScatterable(r)).toBe(true);
    expect(isScatterable({ columns: ["a", "b"], rows: [["x", 1]] })).toBe(false);
    const pts = scatterPoints(r, 200, 100);
    expect(pts).toHaveLength(2);
    expect(pts[0]).toEqual({ cx: 0, cy: 100 }); // (0,0) -> bottom-left
    expect(pts[1]).toEqual({ cx: 200, cy: 0 }); // (max,max) -> top-right
  });

  it("detects and builds a pivot from a 3-column result", () => {
    const r = {
      columns: ["region", "status", "total"],
      rows: [
        ["us", "paid", 10],
        ["us", "refunded", 2],
        ["eu", "paid", 5],
      ],
    };
    expect(isPivotable(r)).toBe(true);
    const p = pivot(r);
    expect(p.columns).toEqual(["paid", "refunded"]);
    const us = p.rows.find((row) => row.label === "us")!;
    expect(us.cells).toEqual([10, 2]);
    const eu = p.rows.find((row) => row.label === "eu")!;
    expect(eu.cells).toEqual([5, null]); // missing combination -> null
  });
});
