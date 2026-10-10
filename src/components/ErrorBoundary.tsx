import { Component, type ErrorInfo, type ReactNode } from "react";
import { ipc } from "../lib/ipc";
import { reportProblem } from "../lib/report";
import s from "./ErrorBoundary.module.css";

const report = (message: string) => ipc.logUiError(message).catch(() => {});

// Errors outside React rendering (event handlers, promises) go to the log as well.
window.addEventListener("error", (e) => report(`${e.message}\n${e.error?.stack ?? ""}`));
window.addEventListener("unhandledrejection", (e) => {
  const r = e.reason;
  report(`Unhandled rejection: ${r instanceof Error ? `${r.message}\n${r.stack ?? ""}` : typeof r === "object" ? JSON.stringify(r) : String(r)}`);
});

/** Keeps a rendering bug from leaving a blank window: explains, logs, and offers a way back. */
export class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null; copied: boolean }> {
  state = { error: null as Error | null, copied: false };

  static getDerivedStateFromError(error: Error) {
    return { error, copied: false };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    report(`${error.message}\n${error.stack ?? ""}\nComponent stack:${info.componentStack ?? ""}`);
  }

  copy = async () => {
    const details = await ipc.diagnostics().catch(() => "");
    await navigator.clipboard.writeText(`${this.state.error?.stack ?? this.state.error?.message}\n\n${details}`).catch(() => {});
    this.setState({ copied: true });
  };

  render() {
    const { error, copied } = this.state;
    if (!error) return this.props.children;
    return (
      <div className={s.screen} role="alert">
        <div className={s.box}>
          <h1 className={s.title}>Something went wrong</h1>
          <p className={s.text}>
            Kiyi hit an unexpected problem and couldn't show this screen. Your saved connections and settings are safe. Reloading usually fixes it; if it keeps happening, please report
            it. The report opens in your browser so you can read it before sending.
          </p>
          <pre className={`${s.detail} selectable`}>{error.message}</pre>
          <div className={s.actions}>
            <button className={s.primary} onClick={() => window.location.reload()}>
              Reload
            </button>
            <button className={s.secondary} onClick={() => reportProblem(`Error: ${error.message.slice(0, 80)}`, error.stack ?? error.message).catch(() => {})}>
              Report this problem
            </button>
            <button className={s.secondary} onClick={this.copy}>
              {copied ? "Copied" : "Copy details"}
            </button>
          </div>
        </div>
      </div>
    );
  }
}
