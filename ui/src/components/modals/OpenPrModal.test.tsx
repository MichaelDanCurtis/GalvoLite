import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { OpenPrModal } from "./OpenPrModal";
import { api } from "../../api";

describe("OpenPrModal", () => {
  it("renders draft on mount", async () => {
    vi.spyOn(api, "getPrDraft").mockResolvedValue({
      title: "Add tests",
      body: "## Task\nfoo\n\n## Commits\n- abc123 init",
      base: "main",
      commits_ahead: 3,
      commits_behind: 0,
    });

    render(<OpenPrModal sessionId="s" open={true} onClose={() => {}} onCreated={() => {}} />);

    await waitFor(() => {
      expect(screen.getByDisplayValue("Add tests")).toBeInTheDocument();
      expect(screen.getByDisplayValue("main")).toBeInTheDocument();
      expect(screen.getByText(/3 commits ahead/i)).toBeInTheDocument();
    });
  });

  it("submit calls openPr with edited values", async () => {
    vi.spyOn(api, "getPrDraft").mockResolvedValue({
      title: "Original", body: "B", base: "main",
      commits_ahead: 1, commits_behind: 0,
    });
    const openPr = vi.fn().mockResolvedValue({
      pr_url: "https://github.com/x/y/pull/1", pr_number: 1,
    });
    vi.spyOn(api, "openPr").mockImplementation(openPr);
    const onCreated = vi.fn();

    render(<OpenPrModal sessionId="s" open={true} onClose={() => {}} onCreated={onCreated} />);

    await screen.findByDisplayValue("Original");
    fireEvent.change(screen.getByDisplayValue("Original"), {
      target: { value: "Edited title" }
    });
    fireEvent.click(screen.getByRole("button", { name: /submit/i }));

    await waitFor(() => {
      expect(openPr).toHaveBeenCalledWith("s", expect.objectContaining({
        title: "Edited title",
        base: "main",
      }));
      expect(onCreated).toHaveBeenCalledWith("https://github.com/x/y/pull/1", 1);
    });
  });

  it("submit disabled when commits_ahead is 0", async () => {
    vi.spyOn(api, "getPrDraft").mockResolvedValue({
      title: "X", body: "B", base: "main",
      commits_ahead: 0, commits_behind: 0,
    });

    render(<OpenPrModal sessionId="s" open={true} onClose={() => {}} onCreated={() => {}} />);

    await screen.findByDisplayValue("X");
    expect(screen.getByRole("button", { name: /submit/i })).toBeDisabled();
  });

  it("submit disabled when title is empty", async () => {
    vi.spyOn(api, "getPrDraft").mockResolvedValue({
      title: "Real", body: "B", base: "main",
      commits_ahead: 1, commits_behind: 0,
    });

    render(<OpenPrModal sessionId="s" open={true} onClose={() => {}} onCreated={() => {}} />);

    await screen.findByDisplayValue("Real");
    fireEvent.change(screen.getByDisplayValue("Real"), { target: { value: "  " } });
    expect(screen.getByRole("button", { name: /submit/i })).toBeDisabled();
  });

  it("error banner renders on openPr rejection", async () => {
    vi.spyOn(api, "getPrDraft").mockResolvedValue({
      title: "X", body: "B", base: "main",
      commits_ahead: 1, commits_behind: 0,
    });
    vi.spyOn(api, "openPr").mockRejectedValue(
      new Error('502: {"error":"git push failed: permission denied"}')
    );

    render(<OpenPrModal sessionId="s" open={true} onClose={() => {}} onCreated={() => {}} />);
    await screen.findByDisplayValue("X");
    fireEvent.click(screen.getByRole("button", { name: /submit/i }));

    await waitFor(() => {
      expect(screen.getByText(/permission denied/i)).toBeInTheDocument();
    });
  });
});
