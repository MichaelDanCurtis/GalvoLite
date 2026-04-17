import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { Modal } from "./Modal";

describe("Modal", () => {
  it("renders children when open", () => {
    render(
      <Modal open onClose={() => {}} title="Test">
        <p>body</p>
      </Modal>,
    );
    expect(screen.getByText("body")).toBeInTheDocument();
    expect(screen.getByText("Test")).toBeInTheDocument();
  });

  it("does not render when closed", () => {
    render(
      <Modal open={false} onClose={() => {}} title="Test">
        <p>body</p>
      </Modal>,
    );
    expect(screen.queryByText("body")).not.toBeInTheDocument();
  });

  it("calls onClose when × is clicked", async () => {
    const onClose = vi.fn();
    render(
      <Modal open onClose={onClose} title="T">
        <p>x</p>
      </Modal>,
    );
    await userEvent.click(screen.getByRole("button", { name: /close/i }));
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("calls onClose on ESC keypress", async () => {
    const onClose = vi.fn();
    render(
      <Modal open onClose={onClose} title="T">
        <p>x</p>
      </Modal>,
    );
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("calls onClose when backdrop clicked", async () => {
    const onClose = vi.fn();
    render(
      <Modal open onClose={onClose} title="T">
        <p>x</p>
      </Modal>,
    );
    await userEvent.click(screen.getByTestId("modal-backdrop"));
    expect(onClose).toHaveBeenCalledOnce();
  });
});
