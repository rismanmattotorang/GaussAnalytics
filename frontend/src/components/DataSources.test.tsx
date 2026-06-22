import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { DataSources } from "./DataSources";

describe("DataSources", () => {
  it("requires an admin token", () => {
    render(<DataSources databases={[]} token={null} onChange={() => {}} />);
    expect(screen.getByText(/administrator/i)).toBeTruthy();
  });

  it("lists sources and offers the add form when signed in", () => {
    render(
      <DataSources
        databases={[
          {
            id: "1",
            name: "Warehouse",
            kind: "postgres",
            is_synced: true,
            created_at: "2026-01-01T00:00:00Z",
          },
        ]}
        token="admin-token"
        onChange={() => {}}
      />,
    );
    // Existing source is listed (its kind appears both in the row and the
    // add-form's kind dropdown, hence getAllByText).
    expect(screen.getByText("Warehouse")).toBeTruthy();
    expect(screen.getAllByText("postgres").length).toBeGreaterThan(0);
    // The add form exposes Test and Add actions.
    expect(screen.getByText("Add a data source")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Test" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Add" })).toBeTruthy();
    // Oracle is selectable as a kind.
    expect(screen.getByRole("option", { name: "oracle" })).toBeTruthy();
  });
});
