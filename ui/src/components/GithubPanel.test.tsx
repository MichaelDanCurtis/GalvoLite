import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { GithubPanel } from "./GithubPanel";
import { api } from "../api";

describe("GithubPanel", () => {
  it("renders empty state when no PRs and no issues", async () => {
    vi.spyOn(api, "listGithubPrs").mockResolvedValue([]);
    vi.spyOn(api, "listGithubIssues").mockResolvedValue([]);

    render(<GithubPanel projectId="p" repo="octocat/hello" />);
    await waitFor(() => {
      expect(screen.getByText(/no open PRs/i)).toBeInTheDocument();
      expect(screen.getByText(/no open issues/i)).toBeInTheDocument();
    });
  });

  it("renders PRs and issues lists", async () => {
    vi.spyOn(api, "listGithubPrs").mockResolvedValue([
      { number: 1, title: "First PR", author: "alice", draft: false,
        updated_at: "2026-05-01T00:00:00Z", url: "https://github.com/x/y/pull/1" }
    ]);
    vi.spyOn(api, "listGithubIssues").mockResolvedValue([
      { number: 2, title: "Bug report", author: "bob", labels: ["bug"],
        updated_at: "2026-05-02T00:00:00Z", url: "https://github.com/x/y/issues/2" }
    ]);

    render(<GithubPanel projectId="p" repo="octocat/hello" />);
    await waitFor(() => {
      expect(screen.getByText("First PR")).toBeInTheDocument();
      expect(screen.getByText("Bug report")).toBeInTheDocument();
    });
  });

  it("Refresh button refetches both lists", async () => {
    const prsMock = vi.spyOn(api, "listGithubPrs").mockResolvedValue([]);
    const issuesMock = vi.spyOn(api, "listGithubIssues").mockResolvedValue([]);

    render(<GithubPanel projectId="p" repo="octocat/hello" />);
    await waitFor(() => expect(prsMock).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(issuesMock).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByRole("button", { name: /refresh/i }));
    await waitFor(() => expect(prsMock).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(issuesMock).toHaveBeenCalledTimes(2));
  });

  it("renders error banner on API error", async () => {
    vi.spyOn(api, "listGithubPrs").mockRejectedValue(new Error("502: rate limited"));
    vi.spyOn(api, "listGithubIssues").mockResolvedValue([]);

    render(<GithubPanel projectId="p" repo="octocat/hello" />);
    await waitFor(() => {
      expect(screen.getByText(/rate limited/i)).toBeInTheDocument();
    });
  });

  it("Start session button calls spawnFromIssue", async () => {
    vi.spyOn(api, "listGithubPrs").mockResolvedValue([]);
    vi.spyOn(api, "listGithubIssues").mockResolvedValue([
      { number: 7, title: "Bug", author: "alice", labels: [], updated_at: "2026-05-01T00:00:00Z", url: "https://github.com/x/y/issues/7" }
    ]);
    const spawn = vi.fn().mockResolvedValue({ session_id: "new-id" });
    vi.spyOn(api, "spawnFromIssue").mockImplementation(spawn);

    render(<GithubPanel projectId="p" repo="x/y" />);
    await screen.findByText("Bug");
    fireEvent.click(screen.getByRole("button", { name: /start session/i }));
    await waitFor(() => {
      expect(spawn).toHaveBeenCalledWith("p", 7);
    });
  });
});
