import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import type { Directive } from "../types";
import { DirectiveTab } from "./DirectiveTab";
import { api } from "../api";

vi.mock("../api", () => ({
  api: {
    getDirective: vi.fn(),
    putDirective: vi.fn(),
  },
}));

const mockGetDirective = vi.mocked(api.getDirective);
const mockPutDirective = vi.mocked(api.putDirective);

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

  it("shows fetch-error state distinct from empty when getDirective rejects", async () => {
    mockGetDirective.mockRejectedValueOnce(new Error("502: bad gateway"));
    render(<DirectiveTab projectId="proj-1" />);
    await waitFor(() =>
      expect(screen.getByText(/couldn't load directive/i)).toBeInTheDocument(),
    );
    expect(screen.getByText(/502: bad gateway/)).toBeInTheDocument();
    // Empty-state Create button must NOT appear: clicking it would risk
    // overwriting an unreachable-but-existing directive.
    expect(screen.queryByRole("button", { name: /create/i })).toBeNull();
    expect(screen.getByRole("button", { name: /retry/i })).toBeInTheDocument();
  });

  it("Save shows inline error on 400", async () => {
    mockGetDirective.mockResolvedValueOnce(null);
    mockPutDirective.mockRejectedValueOnce(new Error("400: missing required section"));
    render(<DirectiveTab projectId="proj-1" />);
    await waitFor(() =>
      expect(screen.getByText(/no directive yet/i)).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("button", { name: /create/i }));
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "# Draft" } });
    fireEvent.click(screen.getByRole("button", { name: /save/i }));
    await waitFor(() =>
      expect(screen.getByText(/400: missing required section/i)).toBeInTheDocument(),
    );
  });

  it("Save coerces non-Error rejection to a string instead of 'undefined'", async () => {
    mockGetDirective.mockResolvedValueOnce(null);
    // Backend client could in principle throw a non-Error; ensure we surface
    // a readable string rather than reading `.message` off `undefined`.
    mockPutDirective.mockRejectedValueOnce("raw rejection");
    render(<DirectiveTab projectId="proj-1" />);
    await waitFor(() =>
      expect(screen.getByText(/no directive yet/i)).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("button", { name: /create/i }));
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "# Draft" } });
    fireEvent.click(screen.getByRole("button", { name: /save/i }));
    await waitFor(() =>
      expect(screen.getByText(/raw rejection/i)).toBeInTheDocument(),
    );
  });

  it("Save then refetches", async () => {
    mockGetDirective.mockResolvedValueOnce(null);
    mockPutDirective.mockResolvedValueOnce(undefined);
    mockGetDirective.mockResolvedValueOnce(stubDirective);

    render(<DirectiveTab projectId="proj-1" />);
    await waitFor(() =>
      expect(screen.getByText(/no directive yet/i)).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("button", { name: /create/i }));
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "# Alpha Project" } });
    fireEvent.click(screen.getByRole("button", { name: /save/i }));
    await waitFor(() =>
      expect(screen.getByText("Alpha Project")).toBeInTheDocument(),
    );
  });
});
