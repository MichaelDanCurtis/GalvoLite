import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { OperatorPromptModal } from "./OperatorPromptModal";

describe("OperatorPromptModal", () => {
  it("renders question + choices and submits selection", async () => {
    const onAnswer = vi.fn().mockResolvedValue(undefined);
    render(<OperatorPromptModal
      prompt={{ prompt_id:"p1", session_id:"s1", question:"go?", choices:["yes","no"], timeout_secs:60 }}
      onAnswer={onAnswer}
    />);
    fireEvent.click(screen.getByRole("button", { name: "yes" }));
    expect(onAnswer).toHaveBeenCalledWith("p1","yes");
  });
  it("renders a freeform input when no choices", () => {
    const onAnswer = vi.fn();
    render(<OperatorPromptModal
      prompt={{ prompt_id:"p1", session_id:"s1", question:"why?", choices:null, timeout_secs:null }}
      onAnswer={onAnswer}
    />);
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "because" } });
    fireEvent.click(screen.getByRole("button", { name: /submit/i }));
    expect(onAnswer).toHaveBeenCalledWith("p1","because");
  });
  it("submit button is disabled when textarea is empty", () => {
    const onAnswer = vi.fn();
    render(<OperatorPromptModal
      prompt={{ prompt_id:"p1", session_id:"s1", question:"why?", choices:null, timeout_secs:null }}
      onAnswer={onAnswer}
    />);
    expect(screen.getByRole("button", { name: /submit/i })).toBeDisabled();
  });
  it("double-clicking a choice button only fires onAnswer once", () => {
    const onAnswer = vi.fn().mockResolvedValue(undefined);
    render(<OperatorPromptModal
      prompt={{ prompt_id:"p1", session_id:"s1", question:"go?", choices:["yes","no"], timeout_secs:60 }}
      onAnswer={onAnswer}
    />);
    const yesBtn = screen.getByRole("button", { name: "yes" });
    fireEvent.click(yesBtn);
    fireEvent.click(yesBtn);
    expect(onAnswer).toHaveBeenCalledTimes(1);
  });
});
