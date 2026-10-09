import { useCallback, useEffect, useState } from "react";
import { CommandPalette } from "./components/CommandPalette";
import { ConnectionDialog } from "./components/ConnectionDialog";
import { type ConnectInit, ConnectionsHome, Overview, Welcome } from "./components/Home";
import { QueryPane } from "./components/QueryPane";
import { Sidebar } from "./components/Sidebar";
import { TabBar } from "./components/TabBar";
import { TableView } from "./components/TableView";
import { SettingsDialog } from "./components/SettingsDialog";
import { Toasts } from "./components/Toasts";
import { UpdateNotice } from "./components/UpdateNotice";
import type { ConnectionConfig } from "./lib/types";
import { useCatalog } from "./state/catalog";
import { useActiveConnection, useConnections } from "./state/connections";
import { applyTheme, useSettings } from "./state/settings";
import { useUi } from "./state/ui";
import { useTabs } from "./state/tabs";
import s from "./App.module.css";
import { isMod } from "./lib/platform";

export function App() {
  const loadConnections = useConnections((st) => st.load);
  const loaded = useConnections((st) => st.loaded);
  const connections = useConnections((st) => st.connections);
  const activeConn = useActiveConnection();
  const tabs = useTabs((st) => st.tabs);
  const activeTabId = useTabs((st) => st.activeId);
  const { open, close, focus } = useTabs.getState();

  const [dialog, setDialog] = useState<{ editing: ConnectionConfig | null; init?: ConnectInit } | null>(null);
  const theme = useSettings((st) => st.theme);
  const live = useConnections((st) => (activeConn ? st.live[activeConn.id] : undefined));

  useEffect(() => {
    loadConnections();
    useCatalog.getState().load();
  }, [loadConnections]);

  useEffect(() => applyTheme(theme), [theme]);

  useEffect(() => {
    const root = document.documentElement;
    if (activeConn) root.dataset.env = activeConn.env;
    else delete root.dataset.env;
  }, [activeConn]);

  const newTab = useCallback(() => {
    const connectionId = useConnections.getState().activeId;
    if (!connectionId) return setDialog({ editing: null });
    open({ connectionId });
  }, [open]);

  const openSql = useCallback(
    (sql: string) => {
      const t = useTabs.getState().tabs.find((x) => x.id === useTabs.getState().activeId);
      open({ connectionId: t?.connectionId ?? useConnections.getState().activeId, sql });
    },
    [open],
  );

  // Global shortcuts. Editor and table shortcuts (⌘↵, ⌘S, ⌘Z…) live in those components.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!isMod(e)) return;
      const k = e.key.toLowerCase();
      const st = useTabs.getState();
      if (k === "k") useUi.getState().setPalette(!useUi.getState().palette);
      else if (k === "t") newTab();
      else if (k === "n") setDialog({ editing: null });
      else if (k === "w" && st.activeId) close(st.activeId);
      else if (k === ",") useUi.getState().openSettings();
      else if (k === "i") {
        const settings = useSettings.getState();
        settings.set({ inspectorOpen: !settings.inspectorOpen });
      }
      else if (k === "r" || k === "p" || k === "=" || k === "-" || k === "0") {
        // Browser reload / print / zoom don't belong in a desktop app.
      } else if (/^[1-9]$/.test(k)) {
        const mine = st.tabs.filter((t) => t.connectionId === useConnections.getState().activeId);
        const target = k === "9" ? mine[mine.length - 1] : mine[Number(k) - 1];
        if (target) focus(target.id);
      } else return;
      e.preventDefault();
    };
    const onContextMenu = (e: MouseEvent) => {
      const el = e.target as HTMLElement;
      if (!el.closest("input, textarea, .selectable, .cm-content")) e.preventDefault();
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("contextmenu", onContextMenu);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("contextmenu", onContextMenu);
    };
  }, [newTab, close, focus]);

  const activeTab = tabs.find((t) => t.id === activeTabId);

  return (
    <div className={s.app}>
      <Sidebar onNewConnection={() => setDialog({ editing: null })} onEdit={(c) => setDialog({ editing: c })} onOpenSql={openSql} />

      <main className={s.main} data-writable={activeConn && !activeConn.readOnly ? true : undefined}>
        <TabBar onNewTab={newTab} />

        {/* Every tab stays mounted so editor history and unsaved edits survive switching. */}
        {tabs.map((t) => (
          <div key={t.id} className={s.pane} hidden={t.id !== activeTabId}>
            {t.kind === "query" ? <QueryPane tab={t} /> : <TableView tab={t} active={t.id === activeTabId} onOpenSql={openSql} />}
          </div>
        ))}

        {!activeTab &&
          (loaded && connections.length === 0 ? (
            <Welcome onConnect={(init) => setDialog({ editing: null, init })} />
          ) : activeConn && live?.status === "connected" ? (
            <Overview />
          ) : !activeConn || live?.status === "error" ? (
            <ConnectionsHome onNew={() => setDialog({ editing: null })} />
          ) : (
            <div className={s.empty}>
              <p className={s.lede}>Connecting to {activeConn.name}…</p>
            </div>
          ))}
      </main>

      <ConnectionDialog isOpen={dialog !== null} editing={dialog?.editing ?? null} init={dialog?.init} onClose={() => setDialog(null)} />
      <SettingsDialog />
      <CommandPalette onNewConnection={() => setDialog({ editing: null })} onOpenSql={openSql} />
      <UpdateNotice />
      <Toasts />
    </div>
  );
}
