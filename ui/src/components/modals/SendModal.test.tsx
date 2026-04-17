import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { SendModal } from "./SendModal";

describe("SendModal", () => {
  it("sends typed text", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    render(<SendModal open onClose={() => {}} onSubmit={onSubmit} />);
    await userEvent.type(
      screen.getByRole("textbox"),
      "please summarise the plan",
    );
    await userEvent.click(screen.getByRole("button", { name: /send/i }));
    expect(onSubmit).toHaveBeenCalledWith("please summarise the plan");
  });

  it("disables send when empty", () => {
    render(<SendModal open onClose={() => {}} onSubmit={vi.fn()} />);
    expect(screen.getByRole("button", { name: /send/i })).toBeDisabled();
  });
});
