import { render, screen, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import type { Directive } from "../types";
import { SpawnControls } from "./SpawnControls";
import { api } from "../api";

vi.mock("../api", () => ({
  api: {
    getPlan: vi.fn(),
    getDirective: vi.fn(),
  },
}));

const mockGetPlan = vi.mocked(api.getPlan);
const mockGetDirective = vi.mocked(api.getDirective);

const stubDirective: Directive = {
  name: "X",
  plan_path: "plan.md",
  phases: [],
  escalation_rules: [],
  notes: "",
  raw_markdown: "# X",
};

beforeEach(() => {
  vi.clearAllMocks();
});

describe("SpawnControls", () => {
  it("disabled when directive missing", async () => {
    mockGetPlan.mockResolvedValueOnce({ markdown: "## Parts\n- 1" });
    mockGetDirective.mockResolvedValueOnce(null);
    render(<SpawnControls projectId="p1" />);
    const btn = await screen.findByRole("button", { name: /spawn sessions/i });
    await waitFor(() => expect(btn).toBeDisabled());
    expect(screen.getByText(/directive\.md required/i)).toBeInTheDocument();
  });

  it("disabled when plan missing", async () => {
    mockGetPlan.mockResolvedValueOnce(null);
    mockGetDirective.mockResolvedValueOnce(stubDirective);
    render(<SpawnControls projectId="p1" />);
    const btn = await screen.findByRole("button", { name: /spawn sessions/i });
    await waitFor(() => expect(btn).toBeDisabled());
    expect(screen.getByText(/plan\.md required/i)).toBeInTheDocument();
  });

  it("disabled when both missing", async () => {
    mockGetPlan.mockResolvedValueOnce(null);
    mockGetDirective.mockResolvedValueOnce(null);
    render(<SpawnControls projectId="p1" />);
    const btn = await screen.findByRole("button", { name: /spawn sessions/i });
    await waitFor(() => expect(btn).toBeDisabled());
    expect(screen.getByText(/plan\.md required/i)).toBeInTheDocument();
    expect(screen.getByText(/directive\.md required/i)).toBeInTheDocument();
  });

  it("enabled when both present", async () => {
    mockGetPlan.mockResolvedValueOnce({ markdown: "## Parts\n- 1" });
    mockGetDirective.mockResolvedValueOnce(stubDirective);
    render(<SpawnControls projectId="p1" />);
    const btn = await screen.findByRole("button", { name: /spawn sessions/i });
    await waitFor(() => expect(btn).toBeEnabled());
  });

  it("calls onSpawn on click when enabled", async () => {
    mockGetPlan.mockResolvedValueOnce({ markdown: "## Parts\n- 1" });
    mockGetDirective.mockResolvedValueOnce(stubDirective);
    const onSpawn = vi.fn();
    render(<SpawnControls projectId="p1" onSpawn={onSpawn} />);
    const btn = await screen.findByRole("button", { name: /spawn sessions/i });
    await waitFor(() => expect(btn).toBeEnabled());
    btn.click();
    expect(onSpawn).toHaveBeenCalledTimes(1);
  });
});
