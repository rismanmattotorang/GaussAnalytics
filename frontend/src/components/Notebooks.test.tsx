import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { Notebooks } from "./Notebooks";
import { api, type Notebook } from "../api/client";

vi.mock("../api/client", () => ({
  api: {
    notebooks: vi.fn(),
    runCell: vi.fn(),
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
    render(<Notebooks token={null} />);
    expect(screen.getByText(/sign in to create and run notebooks/i)).toBeTruthy();
  });

  it("lists notebooks and opens one into a cell editor", async () => {
    render(<Notebooks token="admin" />);
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
    render(<Notebooks token="admin" />);
    await waitFor(() => expect(screen.getByText("Analysis")).toBeTruthy());
    fireEvent.click(screen.getByText("Analysis"));

    fireEvent.click(screen.getByRole("button", { name: "Run" }));
    await waitFor(() => expect(screen.getByText("hello")).toBeTruthy());
    expect(api.runCell).toHaveBeenCalledWith("nb-1", "1 + 1", "admin");
  });
});
