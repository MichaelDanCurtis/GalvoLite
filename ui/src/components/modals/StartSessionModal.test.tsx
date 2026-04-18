import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StartSessionModal } from "./StartSessionModal";

describe("StartSessionModal", () => {
  it("submits task + claude command without a role field", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    render(
      <StartSessionModal
        open
        onClose={() => {}}
        onSubmit={onSubmit}
        projectId="proj-1"
      />,
    );
    await userEvent.type(screen.getByLabelText(/task/i), "do thing");
    await userEvent.click(screen.getByRole("button", { name: /^start$/i }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    expect(onSubmit).toHaveBeenCalledWith({
      project_id: "proj-1",
      task: "do thing",
      claude_cmd: "claude",
    });
  });

  it("disables Start until task is non-empty", async () => {
    render(
      <StartSessionModal
        open
        onClose={() => {}}
        onSubmit={vi.fn()}
        projectId="proj-1"
      />,
    );
    const start = screen.getByRole("button", { name: /^start$/i });
    expect(start).toBeDisabled();
    await userEvent.type(screen.getByLabelText(/task/i), "x");
    expect(start).toBeEnabled();
  });
});
