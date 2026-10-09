import { create } from "zustand";
import { errorMessage, ipc } from "../lib/ipc";
import type { ConnectionConfig, SchemaSnapshot } from "../lib/types";
import { useSettings } from "./settings";
import { useHistory } from "./history";
import { useTablePrefs } from "./tablePrefs";
import { useTabs } from "./tabs";

type Status = "idle" | "connecting" | "connected" | "error";

interface LiveState {
  status: Status;
  serverVersion?: string;
  error?: string;
  schema?: SchemaSnapshot;
  schemaLoading?: boolean;
}

interface ConnectionsState {
  connections: ConnectionConfig[];
  loaded: boolean;
  activeId: string | null;
  live: Record<string, LiveState>;
  load(): Promise<void>;
  save(config: ConnectionConfig, password: string | null, tunnelSecret?: string | null): Promise<ConnectionConfig>;
  remove(id: string): Promise<void>;
  activate(id: string): Promise<void>;
  disconnect(id: string): Promise<void>;
  refreshSchema(id: string): Promise<void>;
}

export const useConnections = create<ConnectionsState>((set, get) => {
  const patch = (id: string, p: Partial<LiveState>) =>
    set((s) => ({ live: { ...s.live, [id]: { ...(s.live[id] ?? { status: "idle" }), ...p } } }));

  return {
    connections: [],
    loaded: false,
    activeId: null,
    live: {},

    async load() {
      const connections = await ipc.listConnections();
      set({ connections, loaded: true });
      // Pick up where the user left off.
      const last = useSettings.getState().lastConnectionId;
      if (!get().activeId && last && connections.some((c) => c.id === last)) get().activate(last);
    },

    async save(config, password, tunnelSecret = null) {
      const saved = await ipc.saveConnection(config, password, tunnelSecret);
      set((s) => {
        const exists = s.connections.some((c) => c.id === saved.id);
        return {
          connections: exists ? s.connections.map((c) => (c.id === saved.id ? saved : c)) : [...s.connections, saved],
        };
      });
      // Settings like read-only apply on connect, so reconnect an open pool.
      if (get().live[saved.id]?.status === "connected") {
        await get().disconnect(saved.id);
        await get().activate(saved.id);
      }
      return saved;
    },

    async remove(id) {
      await ipc.deleteConnection(id);
      useTablePrefs.getState().forgetConnection(id);
      useHistory.getState().forgetConnection(id);
      // Its tabs (and their saved SQL) go with it.
      useTabs.setState((t) => {
        const tabs = t.tabs.filter((tab) => tab.connectionId !== id);
        return { tabs, activeId: tabs.some((tab) => tab.id === t.activeId) ? t.activeId : null };
      });
      set((s) => {
        const { [id]: _, ...live } = s.live;
        return {
          connections: s.connections.filter((c) => c.id !== id),
          live,
          activeId: s.activeId === id ? null : s.activeId,
        };
      });
    },

    async activate(id) {
      if (get().activeId !== id) {
        // Show this connection's work: its most recent tab, or its overview.
        const mine = useTabs.getState().tabs.filter((t) => t.connectionId === id);
        useTabs.setState({ activeId: mine[mine.length - 1]?.id ?? null });
      }
      set({ activeId: id });
      useSettings.getState().set({ lastConnectionId: id });
      const current = get().live[id]?.status;
      if (current === "connected" || current === "connecting") return;
      patch(id, { status: "connecting", error: undefined });
      try {
        const { serverVersion } = await ipc.connect(id);
        patch(id, { status: "connected", serverVersion });
        await get().refreshSchema(id);
      } catch (e) {
        patch(id, { status: "error", error: errorMessage(e) });
      }
    },

    async disconnect(id) {
      await ipc.disconnect(id);
      patch(id, { status: "idle", schema: undefined, serverVersion: undefined });
    },

    async refreshSchema(id) {
      patch(id, { schemaLoading: true });
      try {
        patch(id, { schema: await ipc.loadSchema(id), schemaLoading: false });
      } catch (e) {
        patch(id, { schemaLoading: false, error: errorMessage(e) });
      }
    },
  };
});

export const useActiveConnection = () =>
  useConnections((s) => s.connections.find((c) => c.id === s.activeId) ?? null);
