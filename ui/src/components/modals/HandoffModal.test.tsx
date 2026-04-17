import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { HandoffModal } from "./HandoffModal";

describe("HandoffModal", () => {
  it("calls onConfirm when Trigger clicked", async () => {
    const onConfirm = vi.fn().mockResolvedValue(undefined);
    render(<HandoffModal open onClose={() => {}} onConfirm={onConfirm} />);
    await userEvent.click(screen.getByRole("button", { name: /trigger/i }));
    expect(onConfirm).toHaveBeenCalledOnce();
  });

  it("shows error inline", async () => {
    const onConfirm = vi.fn().mockRejectedValue(new Error("409: not running"));
    render(<HandoffModal open onClose={() => {}} onConfirm={onConfirm} />);
    await userEvent.click(screen.getByRole("button", { name: /trigger/i }));
    expect(await screen.findByText(/not running/)).toBeInTheDocument();
  });
});
