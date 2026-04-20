import { useCallback, useState } from "react";
import { Dashboard } from "./components/Dashboard";
import { OperatorPromptModal } from "./components/OperatorPromptModal";
import { api } from "./api";
import type { DomainEvent } from "./types";

type OperatorPromptPayload = Extract<DomainEvent, { type: "operator.prompt" }>;

export default function App() {
  const [prompt, setPrompt] = useState<OperatorPromptPayload | null>(null);
  const handleGlobalEvent = useCallback((e: DomainEvent) => {
    if (e.type === "operator.prompt") setPrompt(e);
  }, []);
  return (
    <>
      <Dashboard onGlobalEvent={handleGlobalEvent} />
      {prompt && (
        <OperatorPromptModal
          prompt={{
            prompt_id: prompt.prompt_id,
            session_id: prompt.session_id,
            question: prompt.question,
            choices: prompt.choices,
            timeout_secs: prompt.timeout_secs,
          }}
          onAnswer={async (id, answer) => {
            await api.answerOperatorPrompt(id, answer);
            setPrompt(null);
          }}
        />
      )}
    </>
  );
}
