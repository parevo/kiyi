import { create } from "zustand";

/** One run from the SQL editor. */
export interface HistoryEntry {
  id: string;
  connectionId: string;
  sql: string;
  at: number;
  ok: boolean;
  rows: number | null;
  ms: number | null;
}

/** A query the user named and kept. */
export interface SavedQuery {
  id: string;
  name: string;
  sql: string;
  /** The connection it was saved from; `null` = any. */
  connectionId: string | null;
  updatedAt: number;
}

const KEY = "kiyi.queryHistory";
const MAX_ENTRIES = 1_000;
/** Longer scripts are kept, but cut, so history can't fill browser storage. */
const MAX_SQL = 20_000;

const newId = () => `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`;

function read(): { entries: HistoryEntry[]; saved: SavedQuery[] } {
  try {
    const p = JSON.parse(localStorage.getItem(KEY) ?? "{}");
    return { entries: Array.isArray(p.entries) ? p.entries : [], saved: Array.isArray(p.saved) ? p.saved : [] };
  } catch {
    return { entries: [], saved: [] };
  }
}

interface QueryHistory {
  entries: HistoryEntry[];
  saved: SavedQuery[];
  record(e: Omit<HistoryEntry, "id" | "at">): void;
  clear(connectionId: string): void;
  save(q: { name: string; sql: string; connectionId: string | null }): SavedQuery;
  remove(id: string): void;
  forgetConnection(connectionId: string): void;
}

export const useHistory = create<QueryHistory>((set, get) => {
  const persist = () => {
    const { entries, saved } = get();
    try {
      localStorage.setItem(KEY, JSON.stringify({ entries, saved }));
    } catch {
      // Storage full: drop the older half of the history and try once more.
      set({ entries: entries.slice(0, Math.floor(entries.length / 2)) });
      try {
        localStorage.setItem(KEY, JSON.stringify({ entries: get().entries, saved }));
      } catch {
        /* history just won't persist */
      }
    }
  };
  return {
    ...read(),
    record(e) {
      const sql = e.sql.trim();
      if (!sql) return;
      const entry: HistoryEntry = { ...e, sql: sql.length > MAX_SQL ? sql.slice(0, MAX_SQL) : sql, id: newId(), at: Date.now() };
      // Running the same thing again moves it to the top instead of repeating it.
      set((s) => ({ entries: [entry, ...s.entries.filter((x) => !(x.connectionId === e.connectionId && x.sql === entry.sql))].slice(0, MAX_ENTRIES) }));
      persist();
    },
    clear(connectionId) {
      set((s) => ({ entries: s.entries.filter((e) => e.connectionId !== connectionId) }));
      persist();
    },
    save(q) {
      const existing = get().saved.find((s) => s.name === q.name && s.connectionId === q.connectionId);
      const saved: SavedQuery = { ...q, id: existing?.id ?? newId(), updatedAt: Date.now() };
      set((s) => ({ saved: [saved, ...s.saved.filter((x) => x.id !== saved.id)] }));
      persist();
      return saved;
    },
    remove(id) {
      set((s) => ({ saved: s.saved.filter((x) => x.id !== id) }));
      persist();
    },
    forgetConnection(connectionId) {
      set((s) => ({ entries: s.entries.filter((e) => e.connectionId !== connectionId), saved: s.saved.filter((q) => q.connectionId !== connectionId) }));
      persist();
    },
  };
});
