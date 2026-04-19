import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DirectoryBrowserModal } from "./DirectoryBrowserModal";
import { api } from "../../api";

vi.mock("../../api", () => ({
  api: {
    listDirectory: vi.fn(),
  },
}));

const mockListDirectory = api.listDirectory as unknown as ReturnType<typeof vi.fn>;

describe("DirectoryBrowserModal", () => {
  beforeEach(() => {
    mockListDirectory.mockReset();
  });

  it("lists entries for the initial path and lets the user pick the current dir", async () => {
    mockListDirectory.mockResolvedValueOnce({
      path: "/Users/me",
      parent: "/Users",
      entries: [
        { name: "Code", is_dir: true },
        { name: "Documents", is_dir: true },
        { name: "notes.txt", is_dir: false },
      ],
    });

    const onPick = vi.fn();
    render(
      <DirectoryBrowserModal
        open
        initialPath="/Users/me"
        onClose={() => {}}
        onPick={onPick}
      />,
    );

    await waitFor(() => {
      expect(screen.getByText("Code")).toBeInTheDocument();
    });
    expect(screen.getByText("/Users/me")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: /use this folder/i }));
    expect(onPick).toHaveBeenCalledWith("/Users/me");
  });

  it("navigates into a child directory when its row is clicked", async () => {
    mockListDirectory
      .mockResolvedValueOnce({
        path: "/Users/me",
        parent: "/Users",
        entries: [{ name: "Code", is_dir: true }],
      })
      .mockResolvedValueOnce({
        path: "/Users/me/Code",
        parent: "/Users/me",
        entries: [{ name: "GalvoLite", is_dir: true }],
      });

    render(
      <DirectoryBrowserModal
        open
        initialPath="/Users/me"
        onClose={() => {}}
        onPick={() => {}}
      />,
    );

    await waitFor(() => screen.getByText("Code"));
    await userEvent.click(screen.getByText("Code"));
    await waitFor(() => screen.getByText("GalvoLite"));
    expect(mockListDirectory).toHaveBeenCalledWith("/Users/me/Code");
  });

  it("navigates to parent when the up button is clicked", async () => {
    mockListDirectory
      .mockResolvedValueOnce({
        path: "/Users/me",
        parent: "/Users",
        entries: [],
      })
      .mockResolvedValueOnce({
        path: "/Users",
        parent: "/",
        entries: [],
      });

    render(
      <DirectoryBrowserModal
        open
        initialPath="/Users/me"
        onClose={() => {}}
        onPick={() => {}}
      />,
    );

    await waitFor(() => screen.getByText("/Users/me"));
    await userEvent.click(screen.getByRole("button", { name: /up/i }));
    await waitFor(() => screen.getByText("/Users"));
  });
});
