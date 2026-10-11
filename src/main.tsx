import "@fontsource/ibm-plex-sans/400.css";
import "@fontsource/ibm-plex-sans/500.css";
import "@fontsource/ibm-plex-sans/600.css";
import "@fontsource-variable/jetbrains-mono";
import "./styles/global.css";

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { syncUserData } from "./lib/userData";

// In a plain browser during development, talk to kiyi-devbridge instead of Tauri.
if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
  (await import("./dev/bridge")).installBridge();
}

// The stores read their data when first imported, so bring it up to date before that; a slow
// disk mustn't keep the window blank, hence the time limit.
await Promise.race([syncUserData(), new Promise((r) => setTimeout(r, 1500))]);
const { App } = await import("./App");
const { ErrorBoundary } = await import("./components/ErrorBoundary");

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </StrictMode>,
);
