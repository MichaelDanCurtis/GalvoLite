import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { LinkRepoModal } from "./LinkRepoModal";
import { api } from "../../api";

describe("LinkRepoModal", () => {
  it("save success closes modal and calls onSaved", async () => {
    const onSaved = vi.fn();
    const onClose = vi.fn();
    vi.spyOn(api, "linkGithubRepo").mockResolvedValue({ repo: "octocat/hello" });

    render(<LinkRepoModal projectId="p" open={true} onClose={onClose} onSaved={onSaved} />);
    fireEvent.change(screen.getByPlaceholderText(/owner\/name/i), {
      target: { value: "octocat/hello" }
    });
    fireEvent.click(screen.getByRole("button", { name: /save/i }));

    await waitFor(() => {
      expect(api.linkGithubRepo).toHaveBeenCalledWith("p", "octocat/hello");
      expect(onSaved).toHaveBeenCalledWith("octocat/hello");
      expect(onClose).toHaveBeenCalled();
    });
  });

  it("displays inline error on 422 from API", async () => {
    vi.spyOn(api, "linkGithubRepo").mockRejectedValue(
      new Error('422: {"error":"not found or no access"}')
    );

    render(<LinkRepoModal projectId="p" open={true} onClose={() => {}} onSaved={() => {}} />);
    fireEvent.change(screen.getByPlaceholderText(/owner\/name/i), {
      target: { value: "octocat/missing" }
    });
    fireEvent.click(screen.getByRole("button", { name: /save/i }));

    await waitFor(() => {
      expect(screen.getByText(/not found or no access/i)).toBeInTheDocument();
    });
  });

  it("save button disabled when input is empty", () => {
    render(<LinkRepoModal projectId="p" open={true} onClose={() => {}} onSaved={() => {}} />);
    expect(screen.getByRole("button", { name: /save/i })).toBeDisabled();
  });
});
