import React from "react";
import ReactDOM from "react-dom/client";
import { resolveLocale, setLocale } from "../../src/i18n";
import App from "./App";
import "./styles.css";

// The real system-bar insets on Android (MainActivity.kt), as CSS variables
// that styles.css folds into --safe-top and --safe-bottom; and how far in
// from each side the system's Back gesture starts, as --gesture-left and
// --gesture-right.
type InsetsBridge = { get(): string };
function applyInsets() {
  const bridge = (window as unknown as { SourceLightInsets?: InsetsBridge }).SourceLightInsets;
  if (!bridge) return;
  const [top = 0, bottom = 0, left = 0, right = 0] = bridge.get().split(",").map(Number);
  const root = document.documentElement.style;
  root.setProperty("--inset-top", `${top}px`);
  root.setProperty("--inset-bottom", `${bottom}px`);
  root.setProperty("--gesture-left", `${left}px`);
  root.setProperty("--gesture-right", `${right}px`);
}
applyInsets();
window.addEventListener("sourcelight-insets", applyInsets);
window.addEventListener("resize", applyInsets);

// The phone's own language, from the system. The desktop's translations are
// shared; strings not yet translated fall back to English.
void setLocale(resolveLocale("system")).finally(() => {
  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
});
