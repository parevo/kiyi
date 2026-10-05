// Development only: when the UI runs in a plain browser (no Tauri), forward every
// `invoke` to `kiyi-devbridge` so screens can be exercised against real databases.
import { Channel } from "@tauri-apps/api/core";
import { mockIPC } from "@tauri-apps/api/mocks";

const BRIDGE = "/__bridge/invoke/"; // proxied by Vite to kiyi-devbridge

type Internals = { runCallback(id: number, data: unknown): void };

function deliver(channel: Channel<unknown>, messages: unknown[]) {
  const internals = (window as unknown as { __TAURI_INTERNALS__: Internals }).__TAURI_INTERNALS__;
  messages.forEach((message, index) => internals.runCallback(channel.id, { message, index }));
  internals.runCallback(channel.id, { end: true, index: messages.length });
}

export function installBridge() {
  mockIPC(async (cmd, payload) => {
    const args = (payload ?? {}) as Record<string, unknown>;
    if (cmd === "plugin:app|version") return "0.0.0-dev";
    // File dialogs: tests put the path they want on window.__kiyiDialogPath.
    if (cmd === "plugin:dialog|save" || cmd === "plugin:dialog|open") return (window as unknown as { __kiyiDialogPath?: string }).__kiyiDialogPath ?? null;
    if (cmd.startsWith("plugin:")) return null;

    const channel = Object.values(args).find((v): v is Channel<unknown> => v instanceof Channel);
    const body = JSON.stringify(args, (_, v) => (v instanceof Channel ? undefined : v));
    const res = await fetch(BRIDGE + cmd, { method: "POST", headers: { "content-type": "application/json" }, body });
    const data = res.headers.get("content-type")?.includes("json") ? await res.json() : await res.text();
    if (!res.ok) throw data;
    if (channel && Array.isArray(data)) {
      // The bridge returns the whole stream at once; replay it like the app would receive it.
      setTimeout(() => deliver(channel, data), 0);
      return null;
    }
    return data;
  });
}
