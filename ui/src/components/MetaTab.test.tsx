import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { MetaTab } from "./MetaTab";

describe("MetaTab", () => {
  it("renders the four sections", () => {
    render(
      <MetaTab
        session={{
          plan_markdown: "## plan",
          seed_markdown: "## seed",
          handoff_path: "/tmp/handoff-x.md",
          turn_count: 12,
          est_tokens: 9000,
        } as any}
      />,
    );
    expect(screen.getByText(/plan/)).toBeInTheDocument();
    expect(screen.getByText(/seed/)).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: /handoff/i }),
    ).toHaveAttribute("href", "/tmp/handoff-x.md");
    expect(screen.getByText(/12 turns/i)).toBeInTheDocument();
    expect(screen.getByText(/9,000 tokens/i)).toBeInTheDocument();
  });

  it("shows empty state when plan_markdown is null", () => {
    render(<MetaTab session={{ plan_markdown: null } as any} />);
    expect(screen.getByText(/no plan set/i)).toBeInTheDocument();
  });

  it("shows empty state when seed_markdown is null", () => {
    render(<MetaTab session={{ seed_markdown: null } as any} />);
    expect(screen.getByText(/no seed set/i)).toBeInTheDocument();
  });

  it("shows placeholder when handoff_path is absent", () => {
    render(<MetaTab session={{ handoff_path: null } as any} />);
    expect(screen.queryByRole("link", { name: /handoff/i })).toBeNull();
    expect(screen.getByText(/no handoff written/i)).toBeInTheDocument();
  });

  it("defaults turn_count to 0 turns when undefined", () => {
    render(<MetaTab session={{} as any} />);
    expect(screen.getByText(/0 turns/i)).toBeInTheDocument();
  });

  it("defaults est_tokens to 0 tokens when undefined", () => {
    render(<MetaTab session={{} as any} />);
    expect(screen.getByText(/0 tokens/i)).toBeInTheDocument();
  });
});
