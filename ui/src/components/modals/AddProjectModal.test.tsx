import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { AddProjectModal } from "./AddProjectModal";

describe("AddProjectModal", () => {
  it("submits name + path to onSubmit", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();
    render(<AddProjectModal open onClose={onClose} onSubmit={onSubmit} />);

    await userEvent.type(screen.getByLabelText(/name/i), "galvolite");
    await userEvent.type(screen.getByLabelText(/path/i), "/tmp/galvolite");
    await userEvent.click(screen.getByRole("button", { name: /add/i }));

    expect(onSubmit).toHaveBeenCalledWith({
      name: "galvolite",
      path: "/tmp/galvolite",
    });
  });

  it("shows server error inline", async () => {
    const onSubmit = vi.fn().mockRejectedValue(new Error("400: bad path"));
    render(<AddProjectModal open onClose={() => {}} onSubmit={onSubmit} />);

    await userEvent.type(screen.getByLabelText(/name/i), "p");
    await userEvent.type(screen.getByLabelText(/path/i), "/nope");
    await userEvent.click(screen.getByRole("button", { name: /add/i }));

    expect(await screen.findByText(/bad path/)).toBeInTheDocument();
  });

  it("disables submit when fields empty", () => {
    render(<AddProjectModal open onClose={() => {}} onSubmit={vi.fn()} />);
    expect(screen.getByRole("button", { name: /add/i })).toBeDisabled();
  });
});
