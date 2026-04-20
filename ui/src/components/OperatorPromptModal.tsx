import { useState } from "react";
import type { DomainEvent } from "../types";

export type OperatorPrompt = Omit<Extract<DomainEvent, { type: "operator.prompt" }>, "type">;

interface OperatorPromptModalProps {
  prompt: OperatorPrompt;
  onAnswer: (promptId: string, answer: string) => Promise<void>;
}

export function OperatorPromptModal({ prompt, onAnswer }: OperatorPromptModalProps) {
  const [draft, setDraft] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const hasChoices = Array.isArray(prompt.choices) && prompt.choices.length > 0;

  const handleAnswer = async (value: string) => {
    if (submitting) return;
    setSubmitting(true);
    try {
      await onAnswer(prompt.prompt_id, value);
    } catch {
      // Parent handles errors; re-enable so user can retry if the modal survives.
      setSubmitting(false);
    }
    // On success parent unmounts the modal, so state reset is unnecessary.
  };

  return (
    <div className="fixed inset-0 z-[60] bg-black/70 flex items-center justify-center">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="op-prompt-question"
        className="bg-neutral-900 border border-neutral-700 rounded-lg p-6 min-w-[360px] max-w-[560px] shadow-2xl"
      >
        <div className="mb-3 text-xs text-neutral-500">
          Session {prompt.session_id.slice(0, 8)}
        </div>

        <p id="op-prompt-question" className="text-neutral-100 mb-4">{prompt.question}</p>

        {prompt.timeout_secs !== null && (
          <p className="text-xs text-neutral-400 mb-4">
            Times out in {prompt.timeout_secs}s
          </p>
        )}

        {hasChoices ? (
          <div className="flex flex-wrap gap-2">
            {(prompt.choices as string[]).map((choice) => (
              <button
                key={choice}
                type="button"
                className="px-4 py-2 rounded bg-neutral-700 hover:bg-neutral-600 text-neutral-100 text-sm disabled:opacity-40 disabled:cursor-not-allowed"
                disabled={submitting}
                onClick={() => handleAnswer(choice)}
              >
                {choice}
              </button>
            ))}
          </div>
        ) : (
          <div className="flex flex-col gap-3">
            <textarea
              className="w-full rounded bg-neutral-800 border border-neutral-600 text-neutral-100 p-2 text-sm resize-none h-24 focus:outline-none focus:border-neutral-400"
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
            />
            <button
              type="button"
              className="px-4 py-2 rounded bg-blue-700 hover:bg-blue-600 disabled:opacity-40 disabled:cursor-not-allowed text-white text-sm self-end"
              disabled={draft.trim() === "" || submitting}
              onClick={() => handleAnswer(draft)}
            >
              Submit
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
