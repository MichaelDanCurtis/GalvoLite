import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/700.css";
import "./index.css";

// StrictMode intentionally omitted: it double-mounts effects in dev, which
// causes @xterm/addon-webgl@0.19 to crash with `_isDisposed undefined` when
// the term is disposed before WebGL's first frame callback fires. Without
// WebGL we lose solid-block rendering for ░ ▒ ▓ shading chars used by Claude
// Code's progress bars and selected-row highlights — they fall back to the
// font's dotted glyphs. Trade-off accepted: lose StrictMode dev warnings,
// gain correct rendering.
ReactDOM.createRoot(document.getElementById("root")!).render(<App />);
// React import retained for JSX runtime resolution (vite/tsconfig).
void React;
