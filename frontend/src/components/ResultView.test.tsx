import { describe, expect, it } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { ResultView } from "./ResultView";

describe("ResultView", () => {
  it("renders a non-chartable result as a table", () => {
    render(
      <ResultView
        result={{ columns: ["id", "name"], rows: [[1, "ada"], [2, "linus"]] }}
      />,
    );
    expect(screen.getByText("ada")).toBeTruthy();
    expect(screen.getByText("linus")).toBeTruthy();
  });

  it("renders a chartable result with a viz picker including nivo chart kinds", () => {
    render(
      <ResultView result={{ columns: ["status", "value"], rows: [["paid", 15]] }} />,
    );
    // The chart-type picker offers the nivo-backed kinds.
    expect(screen.getByText("bar")).toBeTruthy();
    expect(screen.getByText("line")).toBeTruthy();
    expect(screen.getByText("pie")).toBeTruthy();
    // Switching to the table view surfaces the underlying data.
    fireEvent.click(screen.getByText("table"));
    expect(screen.getByText("paid")).toBeTruthy();
  });
});
