import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { PrReviewsPanel } from "./PrReviewsPanel";
import { api } from "../api";

describe("PrReviewsPanel", () => {
  it("renders empty state when no reviews and no comments", async () => {
    vi.spyOn(api, "getPrReviews").mockResolvedValue({
      reviews: [],
      comments: [],
    });
    render(<PrReviewsPanel sessionId="s" />);
    await waitFor(() => {
      expect(screen.getByText(/no reviews/i)).toBeInTheDocument();
    });
  });

  it("renders reviews and comments", async () => {
    vi.spyOn(api, "getPrReviews").mockResolvedValue({
      reviews: [
        {
          id: 1,
          author: "alice",
          state: "APPROVED",
          body: "LGTM",
          submitted_at: "2026-05-01T00:00:00Z",
          url: "https://github.com/x/y/pull/1#pullrequestreview-1",
        },
      ],
      comments: [
        {
          id: 2,
          author: "bob",
          body: "extract this",
          path: "src/main.rs",
          line: 42,
          url: "https://github.com/x/y/pull/1#discussion_r2",
        },
      ],
    });
    render(<PrReviewsPanel sessionId="s" />);
    await waitFor(() => {
      expect(screen.getByText("@alice")).toBeInTheDocument();
      expect(screen.getByText(/APPROVED/i)).toBeInTheDocument();
      expect(screen.getByText("@bob")).toBeInTheDocument();
      expect(screen.getByText(/src\/main\.rs/)).toBeInTheDocument();
      expect(screen.getByText(/extract this/)).toBeInTheDocument();
    });
  });

  it("Refresh button refetches", async () => {
    const m = vi.spyOn(api, "getPrReviews").mockResolvedValue({
      reviews: [],
      comments: [],
    });
    render(<PrReviewsPanel sessionId="s" />);
    await waitFor(() => expect(m).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole("button", { name: /refresh/i }));
    await waitFor(() => expect(m).toHaveBeenCalledTimes(2));
  });

  it("error banner on api error", async () => {
    vi.spyOn(api, "getPrReviews").mockRejectedValue(
      new Error("502: bad gateway"),
    );
    render(<PrReviewsPanel sessionId="s" />);
    await waitFor(() => {
      expect(screen.getByText(/bad gateway/i)).toBeInTheDocument();
    });
  });
});
