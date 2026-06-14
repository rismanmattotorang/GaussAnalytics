import { describe, expect, it } from "vitest";
import { matchingParam, move, orderedLayout } from "./dashboard";

const base = { id: "d", name: "x", card_ids: ["a", "b", "c"] };

describe("dashboard helpers", () => {
  it("derives layout from card_ids when none saved", () => {
    const l = orderedLayout(base);
    expect(l.map((x) => x.card_id)).toEqual(["a", "b", "c"]);
    expect(l.every((x) => x.w === 1)).toBe(true);
  });

  it("uses the saved layout when present", () => {
    const l = orderedLayout({ ...base, layout: [{ card_id: "c", w: 2 }, { card_id: "a", w: 1 }] });
    expect(l).toEqual([
      { card_id: "c", w: 2 },
      { card_id: "a", w: 1 },
    ]);
  });

  it("matches a parameter by column name", () => {
    expect(matchingParam([{ name: "status" }], "status")).toBe("status");
    expect(matchingParam([{ name: "status" }], "region")).toBeNull();
    expect(matchingParam(undefined, "status")).toBeNull();
  });

  it("moves elements for drag reorder", () => {
    expect(move(["a", "b", "c"], 0, 2)).toEqual(["b", "c", "a"]);
    expect(move(["a", "b", "c"], 2, 0)).toEqual(["c", "a", "b"]);
    expect(move(["a", "b", "c"], 1, 1)).toEqual(["a", "b", "c"]);
  });
});
