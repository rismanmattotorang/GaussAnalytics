import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { Notebooks } from "./Notebooks";
import { api, type Notebook } from "../api/client";

vi.mock("../api/client", () => ({
  api: {
    notebooks: vi.fn(),
    runCell: vi.fn(),
    runOrder: vi.fn(),
    startKernel: vi.fn(),
    stopKernel: vi.fn(),
    updateNotebook: vi.fn(),
    createNotebook: vi.fn(),
    deleteNotebook: vi.fn(),
  },
}));

const sample: Notebook = {
  id: "nb-1",
  name: "Analysis",
  cells: [
    { id: "c1", kind: "markdown", source: "# Title" },
    { id: "c2", kind: "python", source: "1 + 1" },
  ],
  created_at: "2026-01-01T00:00:00Z",
};

beforeEach(() => {
  vi.clearAllMocks();
  (api.notebooks as ReturnType<typeof vi.fn>).mockResolvedValue([sample]);
});

afterEach(cleanup);

describe("Notebooks", () => {
  it("prompts to sign in without a token", async () => {
    render(<Notebooks token={null} databases={[]} />);
    expect(screen.getByText(/sign in to create and run notebooks/i)).toBeTruthy();
  });

  it("lists notebooks and opens one into a cell editor", async () => {
    render(<Notebooks token="admin" databases={[]} />);
    // The list shows the notebook and its cell count.
    await waitFor(() => expect(screen.getByText("Analysis")).toBeTruthy());
    expect(screen.getByText("2 cells")).toBeTruthy();

    fireEvent.click(screen.getByText("Analysis"));
    // Editor shows both cell sources and a Run button for the Python cell.
    expect(screen.getByDisplayValue("# Title")).toBeTruthy();
    expect(screen.getByDisplayValue("1 + 1")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Run" })).toBeTruthy();
  });

  it("runs a Python cell and renders streamed output", async () => {
    (api.runCell as ReturnType<typeof vi.fn>).mockResolvedValue({
      kernel_id: "k1",
      outputs: [{ kind: "stream", name: "stdout", text: "hello\n" }],
    });
    render(<Notebooks token="admin" databases={[]} />);
    await waitFor(() => expect(screen.getByText("Analysis")).toBeTruthy());
    fireEvent.click(screen.getByText("Analysis"));

    fireEvent.click(screen.getByRole("button", { name: "Run" }));
    await waitFor(() => expect(screen.getByText("hello")).toBeTruthy());
    // The whole cell object is sent (kind + source), not just code.
    expect(api.runCell).toHaveBeenCalledWith(
      "nb-1",
      expect.objectContaining({ kind: "python", source: "1 + 1" }),
      "admin",
    );
  });

  it("adds a SQL cell and renders its preview table", async () => {
    (api.notebooks as ReturnType<typeof vi.fn>).mockResolvedValue([
      { ...sample, cells: [{ id: "s1", kind: "sql", source: "select 1 as n", database_id: "db1" }] },
    ]);
    (api.runCell as ReturnType<typeof vi.fn>).mockResolvedValue({
      kernel_id: "k1",
      outputs: [{ kind: "data", data: { "text/plain": "   n\n0  1" } }],
      sql: "select 1 as n",
      preview: { columns: ["n"], rows: [[1]] },
    });
    render(
      <Notebooks
        token="admin"
        databases={[
          { id: "db1", name: "Warehouse", kind: "postgres", is_synced: true, created_at: "x" },
        ]}
      />,
    );
    await waitFor(() => expect(screen.getByText("Analysis")).toBeTruthy());
    fireEvent.click(screen.getByText("Analysis"));

    // The SQL cell exposes a data-source selector and runs into a preview.
    expect(screen.getByRole("combobox", { name: "data source" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Run" }));
    await waitFor(() => expect(screen.getByText("1 row")).toBeTruthy());
    // The preview header column is rendered.
    expect(screen.getByRole("columnheader", { name: "n" })).toBeTruthy();
  });

  it("renders a big-number cell from a fetched DataFrame", async () => {
    (api.notebooks as ReturnType<typeof vi.fn>).mockResolvedValue([
      { ...sample, cells: [{ id: "b1", kind: "bignumber", source: "", input_var: "df" }] },
    ]);
    (api.runCell as ReturnType<typeof vi.fn>).mockResolvedValue({
      kernel_id: "k1",
      outputs: [],
      preview: { columns: ["revenue"], rows: [[1234]] },
    });
    render(<Notebooks token="admin" databases={[]} />);
    await waitFor(() => expect(screen.getByText("Analysis")).toBeTruthy());
    fireEvent.click(screen.getByText("Analysis"));

    fireEvent.click(screen.getByRole("button", { name: "Run" }));
    // The headline value and its label (column name) render.
    await waitFor(() => expect(screen.getByText("1234")).toBeTruthy());
    expect(screen.getByText("revenue")).toBeTruthy();
  });

  it("runs all cells in the server-computed dependency order", async () => {
    (api.runOrder as ReturnType<typeof vi.fn>).mockResolvedValue({ order: ["c2", "c1"] });
    (api.runCell as ReturnType<typeof vi.fn>).mockResolvedValue({ kernel_id: "k1", outputs: [] });
    render(<Notebooks token="admin" databases={[]} />);
    await waitFor(() => expect(screen.getByText("Analysis")).toBeTruthy());
    fireEvent.click(screen.getByText("Analysis"));

    fireEvent.click(screen.getByRole("button", { name: "Run all" }));
    // Run order is requested, then cells execute (markdown c1 is skipped, so
    // only the python cell c2 runs).
    await waitFor(() => expect(api.runOrder).toHaveBeenCalled());
    await waitFor(() =>
      expect(api.runCell).toHaveBeenCalledWith(
        "nb-1",
        expect.objectContaining({ id: "c2" }),
        "admin",
      ),
    );
  });
});
