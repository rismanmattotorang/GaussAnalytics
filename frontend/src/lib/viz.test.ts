import { describe, expect, it } from "vitest";
import { chartData, isChartable, linePoints, pieSlices } from "./viz";

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
});
