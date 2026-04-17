import { useEffect } from "react";
import { motion } from "framer-motion";
import type { Session } from "../types";
import { TerminalPreview } from "./TerminalPreview";
import { SessionRail } from "./SessionRail";

interface Props {
  session: Session;
  onClose: () => void;
  onSend: (origin: DOMRect) => void;
  onHandoff: (origin: DOMRect) => void;
  onEnd: (origin: DOMRect) => void;
  onToggleAutopilot: (next: boolean) => void;
}

export function ExpandedSessionView({
  session,
  onClose,
  onSend,
  onHandoff,
  onEnd,
  onToggleAutopilot,
}: Props) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <motion.div
      data-testid="expanded-backdrop"
      className="fixed inset-0 z-40 bg-black/70 flex items-center justify-center"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      onClick={onClose}
    >
      <motion.div
        className="bg-neutral-950 border border-neutral-800 rounded-lg shadow-2xl flex flex-col"
        style={{ width: "92vw", height: "92vh" }}
        initial={{ scale: 0.85, opacity: 0 }}
        animate={{ scale: 1, opacity: 1 }}
        exit={{ scale: 0.85, opacity: 0 }}
        transition={{ type: "spring", stiffness: 240, damping: 24 }}
        onClick={(e) => e.stopPropagation()}
      >
        <header className="px-4 py-2 border-b border-neutral-800 flex items-center justify-between text-xs text-neutral-300">
          <div>
            <strong className="text-neutral-100">{session.role}</strong>{" "}
            <span className="opacity-50">· {session.id} · {session.state}</span>
          </div>
          <div className="opacity-60">ESC or click outside to close</div>
        </header>
        <div className="flex flex-1 min-h-0">
          <div className="flex-1 min-w-0">
            <TerminalPreview sessionId={session.id} />
          </div>
          <SessionRail
            autopilot={session.autopilot}
            onSend={onSend}
            onHandoff={onHandoff}
            onEnd={onEnd}
            onToggleAutopilot={onToggleAutopilot}
          />
        </div>
      </motion.div>
    </motion.div>
  );
}
