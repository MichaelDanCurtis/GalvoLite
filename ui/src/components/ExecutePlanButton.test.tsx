import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { ExecutePlanButton } from "./ExecutePlanButton";
import { api } from "../api";

vi.mock("../api", () => ({
  api: {
    getPlan: vi.fn(),
    getDirective: vi.fn(),
    executePlan: vi.fn(),
  },
}));

describe("ExecutePlanButton", () => {
  beforeEach(() => vi.clearAllMocks());

  it("disabled when plan or directive missing", async () => {
    (api.getPlan as any).mockResolvedValue(null);
    (api.getDirective as any).mockResolvedValue(null);
    render(<ExecutePlanButton projectId="p1" />);
    const btn = await screen.findByRole("button", { name: /execute plan/i });
    expect(btn).toBeDisabled();
  });

  it("enabled when both present and fires API on click", async () => {
    (api.getPlan as any).mockResolvedValue({ markdown: "x" });
    (api.getDirective as any).mockResolvedValue({ name: "x" });
    (api.executePlan as any).mockResolvedValue({ spawned: ["a", "b"] });
    render(<ExecutePlanButton projectId="p1" />);
    const btn = await screen.findByRole("button", { name: /execute plan/i });
    await waitFor(() => expect(btn).toBeEnabled());
    fireEvent.click(btn);
    await waitFor(() => expect(api.executePlan).toHaveBeenCalledWith("p1"));
  });
});
