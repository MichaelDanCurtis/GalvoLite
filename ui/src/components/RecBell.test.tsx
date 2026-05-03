import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { RecBell } from "./RecBell";
import { api } from "../api";

vi.mock("../api", () => ({
  api: { listRecs: vi.fn() },
}));

describe("RecBell", () => {
  beforeEach(() => vi.clearAllMocks());

  it("renders count badge for pending recs", async () => {
    (api.listRecs as any).mockResolvedValue([
      { id: "r1", severity: "info" }, { id: "r2", severity: "warn" },
    ]);
    render(<RecBell projectId="p1" onOpen={() => {}} degraded={false} />);
    expect(await screen.findByText("2")).toBeInTheDocument();
  });

  it("hides badge when count is 0", async () => {
    (api.listRecs as any).mockResolvedValue([]);
    render(<RecBell projectId="p1" onOpen={() => {}} degraded={false} />);
    expect(await screen.findByLabelText(/recommendations/i)).toBeInTheDocument();
    expect(screen.queryByText(/^[0-9]+$/)).toBeNull();
  });

  it("calls onOpen when clicked", async () => {
    (api.listRecs as any).mockResolvedValue([{ id: "r1", severity: "info" }]);
    const onOpen = vi.fn();
    render(<RecBell projectId="p1" onOpen={onOpen} degraded={false} />);
    const btn = await screen.findByRole("button", { name: /recommendations/i });
    fireEvent.click(btn);
    expect(onOpen).toHaveBeenCalled();
  });

  it("shows degraded styling when prop set", async () => {
    (api.listRecs as any).mockResolvedValue([]);
    const { container } = render(<RecBell projectId="p1" onOpen={() => {}} degraded={true} />);
    expect(container.querySelector(".text-yellow-400, .text-yellow-500")).toBeTruthy();
  });
});
