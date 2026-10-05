import { Button as AriaButton } from "react-aria-components";
import { useActiveConnection, useConnections } from "../state/connections";
import { useTabs } from "../state/tabs";
import { CloseIcon, LockIcon, PlusIcon, Spinner } from "./icons";
import { IconButton } from "./ui";
import s from "./TabBar.module.css";

const ENV_LABEL = { local: "Local", staging: "Staging", production: "Production" } as const;

export function TabBar({ onNewTab }: { onNewTab(): void }) {
  const tabs = useTabs((st) => st.tabs);
  const activeId = useTabs((st) => st.activeId);
  const focus = useTabs((st) => st.focus);
  const close = useTabs((st) => st.close);
  const connections = useConnections((st) => st.connections);
  const active = useActiveConnection();

  return (
    <div className={s.bar} data-tauri-drag-region>
      <div className={s.tabs} role="tablist">
        {tabs.map((t) => {
          const conn = connections.find((c) => c.id === t.connectionId);
          return (
            <div
              key={t.id}
              role="tab"
              aria-selected={t.id === activeId}
              tabIndex={t.id === activeId ? 0 : -1}
              className={s.tab}
              data-active={t.id === activeId || undefined}
              onMouseDown={(e) => {
                if (e.button === 1) close(t.id);
                else focus(t.id);
              }}
              title={conn ? `${t.title} — ${conn.name}` : t.title}
            >
              {t.run?.status === "running" ? (
                <span className={s.running}>
                  <Spinner size={10} />
                </span>
              ) : (
                <span className={s.tabDot} style={{ background: conn ? `var(--env-${conn.env})` : "var(--text-faint)" }} />
              )}
              <span className={s.title}>{t.title}</span>
              <AriaButton aria-label={`${t.title} sekmesini kapat`} className={s.close} onPress={() => close(t.id)}>
                <CloseIcon size={12} />
              </AriaButton>
            </div>
          );
        })}
      </div>
      <IconButton label="Yeni sorgu" shortcut="⌘T" onPress={onNewTab}>
        <PlusIcon />
      </IconButton>
      <div className={s.drag} data-tauri-drag-region />
      {active && (
        <div className={s.right} data-tauri-drag-region>
          <span className={s.envBadge} title={active.readOnly ? "Salt okunur bağlantı" : undefined}>
            {active.readOnly && <LockIcon size={11} />}
            {ENV_LABEL[active.env]}
          </span>
        </div>
      )}
    </div>
  );
}
