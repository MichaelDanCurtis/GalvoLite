import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { RoleBadge } from "./RoleBadge";

describe("RoleBadge", () => {
  it.each([
    ["orchestrator", "ORCH"],
    ["brainstorm", "BRAIN"],
    ["implementation", "IMPL"],
    ["scout", "SCOUT"],
  ])("renders the abbreviation for known role %s", (role, abbr) => {
    render(<RoleBadge role={role} />);
    expect(screen.getByText(abbr)).toBeInTheDocument();
  });

  it("renders unknown role uppercased", () => {
    render(<RoleBadge role="something-weird" />);
    expect(screen.getByText("SOMETHING-WEIRD")).toBeInTheDocument();
  });
});
