import { useRef } from "react";
import { useTerminal } from "../hooks/useTerminal";
import "@xterm/xterm/css/xterm.css";

interface Props {
  sessionId: string;
  mini?: boolean;
  readOnly?: boolean;
}

export function TerminalPreview({ sessionId, mini = false, readOnly = true }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  useTerminal(containerRef, sessionId, { readOnly });

  if (mini) {
    return (
      <div
        className="w-48 h-20 overflow-hidden rounded border border-gray-700 bg-black"
        style={{ transform: "scale(0.4)", transformOrigin: "top left", width: "480px", height: "200px" }}
      >
        <div ref={containerRef} className="w-full h-full" />
      </div>
    );
  }

  return (
    <div className="h-full w-full bg-black overflow-hidden">
      <div ref={containerRef} className="h-full w-full" />
    </div>
  );
}
