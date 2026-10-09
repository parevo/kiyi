import { create } from "zustand";
import { type ColumnLayout, defaultLayout } from "../lib/columnLayout";
import type { Filter, Sort } from "../lib/types";

/** A named way of looking at a table: filters, sort and columns. */
export interface SavedView {
  id: string;
  name: string;
  filters: Filter[];
  rawWhere: string | null;
  search: string | null;
  sort: Sort | null;
  layout: ColumnLayout;
}

interface Prefs {
  layouts: Record<string, ColumnLayout>;
  views: Record<string, SavedView[]>;
}

const KEY = "kiyi.tablePrefs";

function read(): Prefs {
  try {
    const p = JSON.parse(localStorage.getItem(KEY) ?? "{}");
    return { layouts: p.layouts ?? {}, views: p.views ?? {} };
  } catch {
    return { layouts: {}, views: {} };
  }
}

interface TablePrefs extends Prefs {
  layout(table: string): ColumnLayout;
  setLayout(table: string, layout: ColumnLayout): void;
  saveView(table: string, view: Omit<SavedView, "id">): SavedView;
  deleteView(table: string, id: string): void;
  /** Drops everything remembered for a connection's tables (when it's deleted). */
  forgetConnection(connectionId: string): void;
}

/** Per-table column layouts and saved views, keyed by `tableKey`. Kept on this computer. */
export const useTablePrefs = create<TablePrefs>((set, get) => {
  const persist = () => {
    const { layouts, views } = get();
    try {
      localStorage.setItem(KEY, JSON.stringify({ layouts, views }));
    } catch {
      /* storage unavailable; preferences last until restart */
    }
  };
  return {
    ...read(),
    layout: (table) => get().layouts[table] ?? defaultLayout(),
    setLayout(table, layout) {
      set((s) => ({ layouts: { ...s.layouts, [table]: layout } }));
      persist();
    },
    saveView(table, view) {
      const saved = { ...view, id: `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}` };
      set((s) => ({ views: { ...s.views, [table]: [...(s.views[table] ?? []).filter((v) => v.name !== view.name), saved] } }));
      persist();
      return saved;
    },
    deleteView(table, id) {
      set((s) => ({ views: { ...s.views, [table]: (s.views[table] ?? []).filter((v) => v.id !== id) } }));
      persist();
    },
    forgetConnection(connectionId) {
      const keep = <T,>(m: Record<string, T>) => Object.fromEntries(Object.entries(m).filter(([k]) => !k.startsWith(`${connectionId}:`)));
      set((s) => ({ layouts: keep(s.layouts), views: keep(s.views) }));
      persist();
    },
  };
});
