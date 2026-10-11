import { ipc } from "./ipc";

/**
 * The person's own work (history, saved queries, table views, query tabs, settings) lives in
 * files in Kiyi's folder, kept by the core. The web view's storage is only a fast copy for the
 * first paint: a cleared cache loses nothing, and saved queries and views travel with exported
 * connections.
 */
const STORES = [
  ["history", "kiyi.queryHistory"],
  ["table-prefs", "kiyi.tablePrefs"],
  ["query-tabs", "kiyi.queryTabs"],
  ["settings", "kiyi.settings"],
  ["dashboards", "kiyi.dashboards"],
] as const;

export type UserDataName = (typeof STORES)[number][0];

/**
 * Before the stores read their copies: the files win, and data from before Kiyi kept files
 * (only in the web view) is moved into them once.
 */
export async function syncUserData() {
  await Promise.all(
    STORES.map(async ([name, key]) => {
      try {
        const kept = await ipc.loadUserData(name);
        if (kept !== null) localStorage.setItem(key, JSON.stringify(kept));
        else {
          const local = localStorage.getItem(key);
          if (local) await ipc.saveUserData(name, JSON.parse(local));
        }
      } catch {
        /* the web view's copy is used as it is */
      }
    }),
  );
}

const pending = new Map<UserDataName, unknown>();
let timer: ReturnType<typeof setTimeout> | undefined;

function flush() {
  for (const [name, value] of pending) ipc.saveUserData(name, value).catch(() => {});
  pending.clear();
}

/** Saves a store to its file shortly after it changes, and right away when the window goes. */
export function keep(name: UserDataName, value: unknown) {
  pending.set(name, value);
  clearTimeout(timer);
  timer = setTimeout(flush, 300);
}
window.addEventListener("pagehide", flush);

/** Reads a store's file again, after something else (an import) changed it. */
export async function reload(name: UserDataName): Promise<unknown | null> {
  const key = STORES.find(([n]) => n === name)![1];
  const kept = await ipc.loadUserData(name);
  if (kept !== null) localStorage.setItem(key, JSON.stringify(kept));
  return kept;
}
