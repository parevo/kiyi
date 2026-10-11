import { create } from "zustand";
import type { ChartKind } from "../lib/chartData";
import { keep } from "../lib/userData";

/** A chart remembered by column names, so a query whose columns move still draws the same chart. */
export interface SavedChart {
  kind: ChartKind;
  x: string | null;
  y: string[];
}

export interface Tile {
  id: string;
  title: string;
  sql: string;
  chart: SavedChart;
  /** Half the width, or the whole width of the dashboard. */
  wide: boolean;
}

export interface Dashboard {
  id: string;
  connectionId: string;
  name: string;
  tiles: Tile[];
  /** Minutes between automatic refreshes while it's open; 0 = only by hand. */
  refreshMinutes: number;
  updatedAt: number;
}

const KEY = "kiyi.dashboards";
const newId = () => `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`;

function read(): Dashboard[] {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "[]");
    return Array.isArray(v) ? v : [];
  } catch {
    return [];
  }
}

interface Dashboards {
  boards: Dashboard[];
  /** Adds a chart to the named dashboard of this connection, creating it if needed. */
  pin(connectionId: string, dashboardName: string, tile: Omit<Tile, "id" | "wide">): Dashboard;
  update(id: string, f: (d: Dashboard) => Dashboard): void;
  remove(id: string): void;
  forgetConnection(connectionId: string): void;
  replaceAll(boards: Dashboard[]): void;
}

export const useDashboards = create<Dashboards>((set, get) => {
  const persist = () => {
    const boards = get().boards;
    keep("dashboards", boards);
    try {
      localStorage.setItem(KEY, JSON.stringify(boards));
    } catch {
      /* the file in Kiyi's folder still has it */
    }
  };
  return {
    boards: read(),
    pin(connectionId, dashboardName, tile) {
      const name = dashboardName.trim() || "Dashboard";
      const existing = get().boards.find((b) => b.connectionId === connectionId && b.name.toLowerCase() === name.toLowerCase());
      const t: Tile = { ...tile, id: newId(), wide: false };
      const board: Dashboard = existing
        ? { ...existing, tiles: [...existing.tiles, t], updatedAt: Date.now() }
        : { id: newId(), connectionId, name, tiles: [t], refreshMinutes: 0, updatedAt: Date.now() };
      set((s) => ({ boards: existing ? s.boards.map((b) => (b.id === board.id ? board : b)) : [...s.boards, board] }));
      persist();
      return board;
    },
    update(id, f) {
      set((s) => ({ boards: s.boards.map((b) => (b.id === id ? { ...f(b), updatedAt: Date.now() } : b)) }));
      persist();
    },
    remove(id) {
      set((s) => ({ boards: s.boards.filter((b) => b.id !== id) }));
      persist();
    },
    forgetConnection(connectionId) {
      set((s) => ({ boards: s.boards.filter((b) => b.connectionId !== connectionId) }));
      persist();
    },
    replaceAll(boards) {
      set({ boards });
      try {
        localStorage.setItem(KEY, JSON.stringify(boards));
      } catch {
        /* nothing to do */
      }
    },
  };
});
