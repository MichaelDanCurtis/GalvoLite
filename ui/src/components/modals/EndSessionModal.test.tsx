import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { EndSessionModal } from "./EndSessionModal";

describe("EndSessionModal", () => {
  it("calls onConfirm when End clicked", async () => {
    const onConfirm = vi.fn().mockResolvedValue(undefined);
    render(
      <EndSessionModal
        open
        onClose={() => {}}
        onConfirm={onConfirm}
        role="implementation"
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: /^end/i }));
    expect(onConfirm).toHaveBeenCalledOnce();
  });
});
