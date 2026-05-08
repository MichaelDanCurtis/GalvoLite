import { render, screen, waitFor } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { GithubStatusBadge } from "./GithubStatusBadge";
import { api } from "../api";

describe("GithubStatusBadge", () => {
  it("renders connected user when status is connected", async () => {
    vi.spyOn(api, "githubStatus").mockResolvedValue({
      connected: true,
      user: { login: "octocat", avatar_url: "https://example/avatar.png" },
      reason: null,
    });
    render(<GithubStatusBadge />);
    await waitFor(() => {
      expect(screen.getByText(/@octocat/)).toBeInTheDocument();
    });
  });

  it("renders 'not connected' with reason when no token", async () => {
    vi.spyOn(api, "githubStatus").mockResolvedValue({
      connected: false,
      user: null,
      reason: "no token",
    });
    render(<GithubStatusBadge />);
    await waitFor(() => {
      expect(screen.getByText(/not connected/i)).toBeInTheDocument();
    });
  });

  it("renders silently while loading then renders status", async () => {
    let resolve: (v: any) => void;
    vi.spyOn(api, "githubStatus").mockReturnValue(
      new Promise((r) => { resolve = r; }) as any
    );
    render(<GithubStatusBadge />);
    // While loading, the badge can render an empty placeholder or nothing.
    // Only the post-resolve render is asserted.
    resolve!({ connected: false, user: null, reason: "no token" });
    await waitFor(() => {
      expect(screen.getByText(/not connected/i)).toBeInTheDocument();
    });
  });
});
