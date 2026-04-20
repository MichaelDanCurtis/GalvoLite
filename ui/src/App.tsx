import { useCallback, useState } from "react";
import { Dashboard } from "./components/Dashboard";
import { OperatorPromptModal } from "./components/OperatorPromptModal";
import { ToastStack, type Toast } from "./components/ToastStack";
import { api } from "./api";
import type { DomainEvent } from "./types";

type OperatorPromptPayload = Extract<DomainEvent, { type: "operator.prompt" }>;

export default function App() {
  const [prompt, setPrompt] = useState<OperatorPromptPayload | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);

  const handleGlobalEvent = useCallback((e: DomainEvent) => {
    if (e.type === "operator.prompt") {
      setPrompt(e);
    } else if (e.type === "operator.toast") {
      const toast: Toast = {
        id: crypto.randomUUID(),
        level: e.level,
        message: e.message,
      };
      setToasts((prev) => [...prev, toast]);
    }
  }, []);

  const handleDismiss = useCallback((id: string) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
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
            try {
              await api.answerOperatorPrompt(id, answer);
            } catch (err) {
              console.error("operator prompt answer failed:", err);
            } finally {
              setPrompt(null);
            }
          }}
        />
      )}
      <ToastStack toasts={toasts} onDismiss={handleDismiss} />
    </>
  );
}
