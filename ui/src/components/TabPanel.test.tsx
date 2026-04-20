import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { TabPanel } from "./TabPanel";

describe("TabPanel", () => {
  it("renders children when showError is false", () => {
    render(
      <TabPanel
        testId="tab-panel-test"
        showError={false}
        errorTestId="test-error"
        errorText="Could not load"
      >
        <div data-testid="normal-content">hello</div>
      </TabPanel>,
    );
    expect(screen.getByTestId("tab-panel-test")).toBeInTheDocument();
    expect(screen.getByTestId("normal-content")).toBeInTheDocument();
    expect(screen.queryByTestId("test-error")).not.toBeInTheDocument();
  });

  it("renders the error fallback with the given testid and text when showError is true", () => {
    render(
      <TabPanel
        testId="tab-panel-test"
        showError={true}
        errorTestId="test-error"
        errorText="Could not load"
      >
        <div data-testid="normal-content">hello</div>
      </TabPanel>,
    );
    expect(screen.getByTestId("tab-panel-test")).toBeInTheDocument();
    const err = screen.getByTestId("test-error");
    expect(err).toBeInTheDocument();
    expect(err).toHaveTextContent("Could not load");
    expect(screen.queryByTestId("normal-content")).not.toBeInTheDocument();
  });
});
