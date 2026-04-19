import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { RemoveProjectModal } from "./RemoveProjectModal";

describe("RemoveProjectModal", () => {
  it("shows project name + session count", () => {
    render(
      <RemoveProjectModal
        open
        onClose={() => {}}
        onConfirm={vi.fn()}
        projectName="galvolite"
        sessionCount={3}
      />,
    );
    expect(screen.getByText(/galvolite/)).toBeInTheDocument();
    expect(screen.getByText(/3 session/i)).toBeInTheDocument();
  });

  it("calls onConfirm when Remove clicked", async () => {
    const onConfirm = vi.fn().mockResolvedValue(undefined);
    render(
      <RemoveProjectModal
        open
        onClose={() => {}}
        onConfirm={onConfirm}
        projectName="p"
        sessionCount={0}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: /remove/i }));
    expect(onConfirm).toHaveBeenCalledOnce();
  });

  it("shows error inline", async () => {
    const onConfirm = vi.fn().mockRejectedValue(new Error("500: db locked"));
    render(
      <RemoveProjectModal
        open
        onClose={() => {}}
        onConfirm={onConfirm}
        projectName="p"
        sessionCount={1}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: /remove/i }));
    expect(await screen.findByText(/db locked/)).toBeInTheDocument();
  });
});
