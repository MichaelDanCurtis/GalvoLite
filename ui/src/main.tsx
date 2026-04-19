import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/700.css";
import "./index.css";

// StrictMode intentionally omitted: it double-mounts effects in dev, which
// causes @xterm/addon-webgl@0.19 to crash with `_isDisposed undefined` when
// the term is disposed before WebGL's first frame callback fires. We need
// WebGL because it's the only renderer that draws block-element glyphs
// (▀▁▂▃▄▅▆▇█▉▊▋▌▍▎▏▐) as solid filled rects rather than via the font, and
// useTerminal.ts substitutes ▒/░/▓ → █ in incoming bytes to get iTerm-style
// solid selection highlighting. Trade-off: lose StrictMode dev warnings.
ReactDOM.createRoot(document.getElementById("root")!).render(<App />);
// React import retained for JSX runtime resolution (vite/tsconfig).
void React;
