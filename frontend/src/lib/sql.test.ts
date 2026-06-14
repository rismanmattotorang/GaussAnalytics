import { describe, expect, it } from "vitest";
import { extractVars, substituteVars } from "./sql";

describe("sql variables", () => {
  it("extracts distinct variable names in order", () => {
    expect(extractVars("SELECT * FROM t WHERE a = {{a}} AND b = {{ b }} AND c = {{a}}")).toEqual([
      "a",
      "b",
    ]);
    expect(extractVars("SELECT 1")).toEqual([]);
  });

  it("substitutes vars with positional ? and ordered params", () => {
    const r = substituteVars("WHERE a = {{a}} AND b = {{b}} AND a2 = {{a}}", {
      a: "x",
      b: "5",
    });
    expect(r.sql).toBe("WHERE a = ? AND b = ? AND a2 = ?");
    expect(r.params).toEqual(["x", "5", "x"]);
  });

  it("uses empty string for unset variables", () => {
    const r = substituteVars("WHERE a = {{missing}}", {});
    expect(r.params).toEqual([""]);
  });
});
