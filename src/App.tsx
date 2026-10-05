import { useCallback, useEffect, useState } from "react";
import { ConnectionDialog } from "./components/ConnectionDialog";
import { QueryPane } from "./components/QueryPane";
import { Sidebar } from "./components/Sidebar";
import { TabBar } from "./components/TabBar";
import { TableView } from "./components/TableView";
import { Button } from "./components/ui";
import { UpdateNotice } from "./components/UpdateNotice";
import type { ConnectionConfig } from "./lib/types";
import { useActiveConnection, useConnections } from "./state/connections";
import { useTabs } from "./state/tabs";
import s from "./App.module.css";

export function App() {
  const loadConnections = useConnections((st) => st.load);
  const loaded = useConnections((st) => st.loaded);
  const connections = useConnections((st) => st.connections);
  const activeConn = useActiveConnection();
  const tabs = useTabs((st) => st.tabs);
  const activeTabId = useTabs((st) => st.activeId);
  const { open, close, focus } = useTabs.getState();

  const [dialog, setDialog] = useState<{ editing: ConnectionConfig | null } | null>(null);

  useEffect(() => {
    loadConnections();
  }, [loadConnections]);

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
      if (!e.metaKey && !e.ctrlKey) return;
      const k = e.key.toLowerCase();
      const st = useTabs.getState();
      if (k === "t") newTab();
      else if (k === "n") setDialog({ editing: null });
      else if (k === "w" && st.activeId) close(st.activeId);
      else if (k === "r" || k === "p" || k === "=" || k === "-" || k === "0") {
        // Browser reload / print / zoom don't belong in a desktop app.
      } else if (/^[1-9]$/.test(k)) {
        const target = k === "9" ? st.tabs[st.tabs.length - 1] : st.tabs[Number(k) - 1];
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
            <div className={s.empty}>
              <img src="/icon.svg" alt="" className={s.mark} />
              <h1 className={s.headline}>Kıyı'ya hoş geldin</h1>
              <p className={s.lede}>PostgreSQL ya da MySQL bağlantı adresini yapıştır, gerisini Kıyı halletsin.</p>
              <Button variant="primary" onPress={() => setDialog({ editing: null })}>
                İlk bağlantını ekle
              </Button>
            </div>
          ) : (
            <div className={s.empty}>
              <p className={s.lede}>
                {activeConn ? "Soldan bir tablo seç ya da yeni bir sorgu aç." : "Başlamak için soldan bir bağlantı seç."}
              </p>
              <div className={s.shortcuts}>
                <kbd>⌘T</kbd> <span>Yeni sorgu</span>
                <kbd>⌘N</kbd> <span>Yeni bağlantı</span>
                <kbd>⌘↵</kbd> <span>İfadeyi çalıştır</span>
                <kbd>⌘S</kbd> <span>Değişiklikleri kaydet</span>
                <kbd>⌘F</kbd> <span>Tabloyu filtrele</span>
              </div>
              {activeConn && (
                <Button onPress={newTab} variant="primary">
                  Yeni sorgu
                </Button>
              )}
            </div>
          ))}
      </main>

      <ConnectionDialog isOpen={dialog !== null} editing={dialog?.editing ?? null} onClose={() => setDialog(null)} />
      <UpdateNotice />
    </div>
  );
}
