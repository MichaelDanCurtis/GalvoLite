import { useState } from "react";
import { Send, ArrowLeftRight, Zap, Power, ScrollText, ChevronLeft, ChevronRight } from "lucide-react";

interface Props {
  autopilot: boolean;
  onSend: (origin: DOMRect) => void;
  onHandoff: (origin: DOMRect) => void;
  onToggleAutopilot: (next: boolean) => void;
  onEnd: (origin: DOMRect) => void;
  onOpenEvents: () => void;
}

export function SessionRail({
  autopilot,
  onSend,
  onHandoff,
  onToggleAutopilot,
  onEnd,
  onOpenEvents,
}: Props) {
  const [expanded, setExpanded] = useState(false);

  function trigger(cb: (r: DOMRect) => void) {
    return (e: React.MouseEvent) => {
      const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
      cb(rect);
    };
  }

  const iconBtn =
    "flex items-center gap-2 text-xs text-neutral-300 hover:text-neutral-100 rounded px-2 py-2";

  return (
    <aside
      className={`border-l border-neutral-800 bg-neutral-950 flex flex-col transition-[width] duration-200 ${
        expanded ? "w-28" : "w-9"
      }`}
    >
      <button
        className={iconBtn}
        aria-label="Send to session"
        onClick={trigger(onSend)}
      >
        <Send size={16} />
        {expanded && <span>Send</span>}
      </button>
      <button
        className={iconBtn}
        aria-label="Handoff"
        onClick={trigger(onHandoff)}
      >
        <ArrowLeftRight size={16} />
        {expanded && <span>Handoff</span>}
      </button>
      <button
        className={`${iconBtn} ${autopilot ? "text-yellow-300" : ""}`}
        aria-label="Autopilot"
        aria-pressed={autopilot}
        onClick={() => onToggleAutopilot(!autopilot)}
      >
        <Zap size={16} />
        {expanded && <span>Autopilot</span>}
      </button>
      <button
        className={`${iconBtn} text-red-400 hover:text-red-300`}
        aria-label="End session"
        onClick={trigger(onEnd)}
      >
        <Power size={16} />
        {expanded && <span>End session</span>}
      </button>
      <button
        className={iconBtn}
        aria-label="Event log"
        onClick={onOpenEvents}
      >
        <ScrollText size={16} />
        {expanded && <span>Event log</span>}
      </button>
      <div className="flex-1" />
      <button
        className={iconBtn}
        aria-label={expanded ? "Collapse rail" : "Expand rail"}
        onClick={() => setExpanded((v) => !v)}
      >
        {expanded ? <ChevronRight size={16} /> : <ChevronLeft size={16} />}
      </button>
    </aside>
  );
}
