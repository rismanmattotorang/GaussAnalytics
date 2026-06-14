import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
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

  it("renders a chartable result with a viz picker", () => {
    render(
      <ResultView result={{ columns: ["status", "value"], rows: [["paid", 15]] }} />,
    );
    // The category label appears (default bar chart) and a chart-type picker exists.
    expect(screen.getByText("paid")).toBeTruthy();
    expect(screen.getByText("pie")).toBeTruthy();
  });
});
