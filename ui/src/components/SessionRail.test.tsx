import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { SessionRail } from "./SessionRail";

describe("SessionRail", () => {
  const baseProps = {
    autopilot: false,
    onSend: vi.fn(),
    onHandoff: vi.fn(),
    onToggleAutopilot: vi.fn(),
    onEnd: vi.fn(),
  };

  it("renders four action icons", () => {
    render(<SessionRail {...baseProps} />);
    expect(screen.getByRole("button", { name: /send/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /handoff/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /autopilot/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /end session/i })).toBeInTheDocument();
  });

  it("fires onSend when send icon clicked", async () => {
    const onSend = vi.fn();
    render(<SessionRail {...baseProps} onSend={onSend} />);
    await userEvent.click(screen.getByRole("button", { name: /send/i }));
    expect(onSend).toHaveBeenCalledOnce();
  });

  it("fires onToggleAutopilot with opposite state", async () => {
    const onToggle = vi.fn();
    render(
      <SessionRail {...baseProps} autopilot={false} onToggleAutopilot={onToggle} />,
    );
    await userEvent.click(screen.getByRole("button", { name: /autopilot/i }));
    expect(onToggle).toHaveBeenCalledWith(true);
  });

  it("expands when expand handle clicked", async () => {
    render(<SessionRail {...baseProps} />);
    await userEvent.click(screen.getByRole("button", { name: /expand rail/i }));
    expect(screen.getByText(/End session/)).toBeInTheDocument();
  });
});
