import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import type { Directive } from "../types";

vi.mock("../api", () => ({
  api: {
    getDirective: vi.fn(),
    putDirective: vi.fn(),
  },
}));

// Import after mock is registered so the component gets the mocked api.
const { DirectiveTab } = await import("./DirectiveTab");
const { api } = await import("../api");

const mockGetDirective = api.getDirective as ReturnType<typeof vi.fn>;
const mockPutDirective = api.putDirective as ReturnType<typeof vi.fn>;

const stubDirective: Directive = {
  name: "Alpha Project",
  plan_path: "plan.md",
  phases: [
    { id: "P1", parts: [1], autonomy: "full", depends_on: [] },
  ],
  escalation_rules: [
    { trigger: "blocked", action: "notify" },
  ],
  notes: "Some notes here",
  raw_markdown: "# Alpha Project\n## phases\n- P1",
};

beforeEach(() => {
  vi.clearAllMocks();
});

describe("DirectiveTab", () => {
  it("renders directive name + phases", async () => {
    mockGetDirective.mockResolvedValueOnce(stubDirective);
    render(<DirectiveTab projectId="proj-1" />);
    await waitFor(() =>
      expect(screen.getByText("Alpha Project")).toBeInTheDocument(),
    );
    expect(screen.getByText("P1")).toBeInTheDocument();
  });

  it("shows empty state on null", async () => {
    mockGetDirective.mockResolvedValueOnce(null);
    render(<DirectiveTab projectId="proj-1" />);
    await waitFor(() =>
      expect(screen.getByText(/no directive yet/i)).toBeInTheDocument(),
    );
  });

  it("Save shows inline error on 400", async () => {
    mockGetDirective.mockResolvedValueOnce(null);
    mockPutDirective.mockRejectedValueOnce(new Error("400: missing required section"));
    render(<DirectiveTab projectId="proj-1" />);
    // Wait for empty state
    await waitFor(() =>
      expect(screen.getByText(/no directive yet/i)).toBeInTheDocument(),
    );
    // Enter editing mode
    fireEvent.click(screen.getByRole("button", { name: /create/i }));
    // Type something into the textarea
    const textarea = screen.getByRole("textbox");
    fireEvent.change(textarea, { target: { value: "# Draft" } });
    // Click Save
    fireEvent.click(screen.getByRole("button", { name: /save/i }));
    // Error message should appear
    await waitFor(() =>
      expect(screen.getByText(/400: missing required section/i)).toBeInTheDocument(),
    );
  });

  it("Save then refetches", async () => {
    // First getDirective call → null (before save)
    mockGetDirective.mockResolvedValueOnce(null);
    // putDirective resolves successfully
    mockPutDirective.mockResolvedValueOnce(undefined);
    // Second getDirective call (after save) → populated directive
    mockGetDirective.mockResolvedValueOnce(stubDirective);

    render(<DirectiveTab projectId="proj-1" />);
    // Wait for empty state
    await waitFor(() =>
      expect(screen.getByText(/no directive yet/i)).toBeInTheDocument(),
    );
    // Enter editing mode
    fireEvent.click(screen.getByRole("button", { name: /create/i }));
    // Type something
    const textarea = screen.getByRole("textbox");
    fireEvent.change(textarea, { target: { value: "# Alpha Project" } });
    // Click Save
    fireEvent.click(screen.getByRole("button", { name: /save/i }));
    // After successful save + refetch, directive name should appear
    await waitFor(() =>
      expect(screen.getByText("Alpha Project")).toBeInTheDocument(),
    );
  });
});
