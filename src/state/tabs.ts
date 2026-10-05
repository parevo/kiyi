import { create } from "zustand";
import { ipc } from "../lib/ipc";
import type { Cell, ColumnMeta, ErrorInfo, Filter } from "../lib/types";

export interface ResultSet {
  columns: ColumnMeta[];
  /** Mutated in place while streaming; `rowCount` in state is what triggers re-renders. */
  rows: Cell[][];
  rowsAffected: number | null;
}

export interface RunState {
  status: "running" | "done" | "error";
  queryId: string;
  /** Character offset of the executed statement within the editor, to place error markers. */
  offset: number;
  sets: ResultSet[];
  rowCount: number;
  statements: number;
  elapsedMs: number | null;
  startedAt: number;
  cancelled: boolean;
  error: ErrorInfo | null;
}

export type TabKind = "query" | "table" | "create";
export type TableView = "data" | "structure";

export interface Tab {
  id: string;
  kind: TabKind;
  title: string;
  connectionId: string | null;
  sql: string;
  run: RunState | null;
  /** Table tabs: `connectionId:schema.table`, so the tree can focus an open tab. */
  table?: string;
  schema?: string | null;
  tableName?: string;
  view?: TableView;
  /** Filters to apply when a table tab first loads (e.g. following a foreign key). */
  initialFilters?: Filter[];
  /** Unsaved grid or structure edits; closing asks first. */
  dirty?: boolean;
}

export const tableKey = (connectionId: string, schema: string | null, table: string) =>
  `${connectionId}:${schema ?? ""}.${table}`;

interface TabsState {
  tabs: Tab[];
  activeId: string | null;
  open(init: Partial<Tab> & { connectionId: string | null }): string;
  openTable(connectionId: string, schema: string | null, table: string, view?: TableView, filters?: Filter[]): string;
  openCreate(connectionId: string, schema: string | null): string;
  patch(id: string, patch: Partial<Tab>): void;
  /** Closes without asking; returns false when the user cancelled because of unsaved edits. */
  close(id: string, force?: boolean): boolean;
  focus(id: string): void;
  setSql(id: string, sql: string): void;
  execute(id: string, sql: string, offset: number): Promise<void>;
  cancel(id: string): void;
}

let seq = 0;
const newId = () => `${Date.now().toString(36)}-${(seq++).toString(36)}`;

export const useTabs = create<TabsState>((set, get) => {
  const update = (id: string, fn: (t: Tab) => Partial<Tab>) =>
    set((s) => ({ tabs: s.tabs.map((t) => (t.id === id ? { ...t, ...fn(t) } : t)) }));
  const updateRun = (id: string, queryId: string, fn: (r: RunState) => Partial<RunState>) =>
    update(id, (t) => (t.run && t.run.queryId === queryId ? { run: { ...t.run, ...fn(t.run) } } : {}));

  return {
    tabs: [],
    activeId: null,

    open(init) {
      const n = get().tabs.filter((t) => t.kind === "query").length + 1;
      const tab: Tab = { id: newId(), kind: "query", title: `Sorgu ${n}`, sql: "", run: null, ...init };
      set((s) => ({ tabs: [...s.tabs, tab], activeId: tab.id }));
      return tab.id;
    },

    openTable(connectionId, schema, table, view = "data", filters) {
      const key = tableKey(connectionId, schema, table);
      const existing = get().tabs.find((t) => t.table === key);
      // Following a foreign key opens a fresh, filtered tab rather than disturbing an open one.
      if (existing && !filters) {
        set((s) => ({
          activeId: existing.id,
          tabs: s.tabs.map((t) => (t.id === existing.id ? { ...t, view } : t)),
        }));
        return existing.id;
      }
      return get().open({
        kind: "table",
        connectionId,
        table: filters ? undefined : key,
        schema,
        tableName: table,
        title: table,
        view,
        initialFilters: filters,
      });
    },

    openCreate(connectionId, schema) {
      return get().open({ kind: "create", connectionId, schema, title: "Yeni tablo" });
    },

    patch: (id, p) => update(id, () => p),

    close(id, force = false) {
      const tab = get().tabs.find((t) => t.id === id);
      if (tab?.dirty && !force && !confirm(`"${tab.title}" içinde kaydedilmemiş değişiklikler var. Yine de kapatılsın mı?`)) {
        return false;
      }
      if (tab?.run?.status === "running") ipc.cancelQuery(tab.run.queryId);
      set((s) => {
        const index = s.tabs.findIndex((t) => t.id === id);
        const tabs = s.tabs.filter((t) => t.id !== id);
        const activeId = s.activeId === id ? (tabs[Math.min(index, tabs.length - 1)]?.id ?? null) : s.activeId;
        return { tabs, activeId };
      });
      return true;
    },

    focus: (id) => set({ activeId: id }),

    setSql: (id, sql) => update(id, () => ({ sql })),

    async execute(id, sql, offset) {
      const tab = get().tabs.find((t) => t.id === id);
      if (!tab?.connectionId || tab.run?.status === "running") return;
      const queryId = newId();
      update(id, () => ({
        run: {
          status: "running",
          queryId,
          offset,
          sets: [],
          rowCount: 0,
          statements: 0,
          elapsedMs: null,
          startedAt: performance.now(),
          cancelled: false,
          error: null,
        },
      }));

      // Rows arrive in many small batches; coalesce re-renders to one per frame.
      let frame = 0;
      const scheduleRowCount = () => {
        if (frame) return;
        frame = requestAnimationFrame(() => {
          frame = 0;
          updateRun(id, queryId, (r) => ({ rowCount: r.sets.reduce((n, s) => n + s.rows.length, 0) }));
        });
      };

      try {
        await ipc.runQuery(tab.connectionId, queryId, sql, (event) => {
          const run = get().tabs.find((t) => t.id === id)?.run;
          if (!run || run.queryId !== queryId) return;
          switch (event.type) {
            case "columns":
              updateRun(id, queryId, (r) => ({ sets: [...r.sets, { columns: event.columns, rows: [], rowsAffected: null }] }));
              break;
            case "rows": {
              const current = run.sets[run.sets.length - 1];
              if (current) {
                for (const row of event.rows) current.rows.push(row);
                scheduleRowCount();
              }
              break;
            }
            case "statementDone":
              updateRun(id, queryId, (r) => {
                const last = r.sets[r.sets.length - 1];
                // A statement without rows (UPDATE, DDL…) gets its own entry for the summary.
                const sets =
                  last && last.rowsAffected === null
                    ? [...r.sets.slice(0, -1), { ...last, rowsAffected: event.rowsAffected }]
                    : [...r.sets, { columns: [], rows: [], rowsAffected: event.rowsAffected }];
                return { sets, statements: r.statements + 1 };
              });
              break;
            case "error":
              updateRun(id, queryId, () => ({ error: event.error }));
              break;
            case "done":
              cancelAnimationFrame(frame);
              frame = 0;
              updateRun(id, queryId, (r) => ({
                status: r.error ? "error" : "done",
                elapsedMs: event.elapsedMs,
                cancelled: event.cancelled,
                rowCount: r.sets.reduce((n, s) => n + s.rows.length, 0),
              }));
              break;
          }
        });
      } catch (e) {
        const message = e && typeof e === "object" && "message" in e ? String(e.message) : String(e);
        updateRun(id, queryId, () => ({ status: "error", error: { message, code: null, position: null } }));
      }
    },

    cancel(id) {
      const run = get().tabs.find((t) => t.id === id)?.run;
      if (run?.status === "running") ipc.cancelQuery(run.queryId);
    },
  };
});

export const useActiveTab = () => useTabs((s) => s.tabs.find((t) => t.id === s.activeId) ?? null);
