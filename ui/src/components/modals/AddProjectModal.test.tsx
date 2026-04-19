import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AddProjectModal } from "./AddProjectModal";

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));
vi.mock("../../api", () => ({
  api: {
    listDirectory: vi.fn().mockResolvedValue({
      path: "/Users/me",
      parent: "/Users",
      entries: [{ name: "Project", is_dir: true }],
    }),
  },
}));

import { open as tauriOpen } from "@tauri-apps/plugin-dialog";

describe("AddProjectModal", () => {
  beforeEach(() => {
    (tauriOpen as unknown as ReturnType<typeof vi.fn>).mockReset();
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });
  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });

  it("submits name + path", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    render(<AddProjectModal open onClose={() => {}} onSubmit={onSubmit} />);
    await userEvent.type(screen.getByLabelText(/name/i), "demo");
    await userEvent.type(screen.getByLabelText(/path/i), "/tmp/demo");
    await userEvent.click(screen.getByRole("button", { name: /^add$/i }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith({ name: "demo", path: "/tmp/demo" }));
  });

  it("calls Tauri dialog when running inside Tauri", async () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    (tauriOpen as unknown as ReturnType<typeof vi.fn>).mockResolvedValueOnce("/picked/by/tauri");
    render(<AddProjectModal open onClose={() => {}} onSubmit={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: /browse for folder/i }));
    await waitFor(() =>
      expect(screen.getByLabelText(/path/i)).toHaveValue("/picked/by/tauri"),
    );
  });

  it("opens DirectoryBrowserModal when not running inside Tauri", async () => {
    render(<AddProjectModal open onClose={() => {}} onSubmit={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: /browse for folder/i }));
    // The directory browser modal exposes "Use this folder"
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: /use this folder/i }),
      ).toBeInTheDocument(),
    );
  });

  it("surfaces Tauri dialog errors instead of swallowing them", async () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    (tauriOpen as unknown as ReturnType<typeof vi.fn>).mockRejectedValueOnce(
      new Error("dialog plugin not initialized"),
    );
    render(<AddProjectModal open onClose={() => {}} onSubmit={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: /browse for folder/i }));
    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent(/dialog plugin not initialized/i),
    );
  });
});
