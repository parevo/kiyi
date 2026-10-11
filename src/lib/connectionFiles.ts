import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { useConnections } from "../state/connections";
import { useHistory } from "../state/history";
import { useTablePrefs } from "../state/tablePrefs";
import { reload } from "./userData";
import { toast } from "../state/toasts";
import { errorMessage, ipc } from "./ipc";

const plural = (n: number) => (n === 1 ? "1 connection" : `${n} connections`);

/** Saves connections (all when `ids` is empty) to a file for another computer or a teammate. */
export async function exportConnections(ids: string[] = []) {
  const path = await saveDialog({ defaultPath: "kiyi-connections.json", filters: [{ name: "Kiyi connections", extensions: ["json"] }] });
  if (!path) return;
  try {
    const n = await ipc.exportConnections(ids, path);
    toast.success(`Exported ${plural(n)}. Passwords and keys aren't in the file.`);
  } catch (e) {
    toast.error(errorMessage(e));
  }
}

export async function importConnections() {
  const path = await openDialog({ multiple: false, filters: [{ name: "Kiyi connections", extensions: ["json"] }] });
  if (typeof path !== "string") return;
  try {
    const { added, queries, views } = await ipc.importConnections(path);
    if (added.length) useConnections.setState((st) => ({ connections: [...st.connections, ...added] }));
    // Saved queries and views came along into the files; show them.
    const history = (await reload("history")) as { saved?: unknown[] } | null;
    if (history?.saved) useHistory.setState({ saved: history.saved as never });
    const prefs = (await reload("table-prefs")) as { layouts?: object; views?: object } | null;
    if (prefs) useTablePrefs.setState({ layouts: (prefs.layouts ?? {}) as never, views: (prefs.views ?? {}) as never });
    const extra = [queries && `${queries} saved ${queries === 1 ? "query" : "queries"}`, views && `${views} ${views === 1 ? "view" : "views"}`].filter(Boolean).join(" and ");
    if (!added.length && !extra) return toast.info("Nothing new: those connections are already here.");
    toast.success(`Added ${plural(added.length)}${extra ? `, with ${extra}` : ""}.${added.length ? " You'll be asked for passwords when you connect." : ""}`);
  } catch (e) {
    toast.error(errorMessage(e));
  }
}

/** Creates the sample database from scratch and opens it. */
export async function openSample() {
  try {
    toast.info("Setting up the sample database…");
    const sample = await ipc.createSample();
    const st = useConnections.getState();
    useConnections.setState({ connections: st.connections.some((c) => c.id === sample.id) ? st.connections : [...st.connections, sample] });
    // A fresh file replaced the old one, so reconnect rather than reuse the open pool.
    if (st.live[sample.id]) await st.disconnect(sample.id).catch(() => {});
    await useConnections.getState().activate(sample.id);
  } catch (e) {
    toast.error(errorMessage(e));
  }
}
