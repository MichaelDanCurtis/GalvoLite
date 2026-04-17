import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { StartSessionModal } from "./StartSessionModal";

describe("StartSessionModal", () => {
  it("submits task + role + command", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    render(
      <StartSessionModal
        open
        onClose={() => {}}
        onSubmit={onSubmit}
        projectId="proj-1"
      />,
    );
    await userEvent.type(screen.getByLabelText(/task/i), "refactor api");
    const roleInput = screen.getByLabelText(/role/i);
    await userEvent.clear(roleInput);
    await userEvent.type(roleInput, "scout");
    const cmdInput = screen.getByLabelText(/command/i);
    await userEvent.clear(cmdInput);
    await userEvent.type(cmdInput, "claude");
    await userEvent.click(screen.getByRole("button", { name: /start/i }));

    expect(onSubmit).toHaveBeenCalledWith({
      project_id: "proj-1",
      task: "refactor api",
      role: "scout",
      claude_cmd: "claude",
    });
  });

  it("disables start with empty task", () => {
    render(
      <StartSessionModal
        open
        onClose={() => {}}
        onSubmit={vi.fn()}
        projectId="p"
      />,
    );
    expect(screen.getByRole("button", { name: /start/i })).toBeDisabled();
  });
});
