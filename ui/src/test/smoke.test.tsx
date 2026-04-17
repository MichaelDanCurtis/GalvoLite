import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";

describe("test harness", () => {
  it("renders and queries DOM", () => {
    render(<div>hello</div>);
    expect(screen.getByText("hello")).toBeInTheDocument();
  });
});
