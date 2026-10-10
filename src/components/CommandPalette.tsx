import { useEffect, useMemo, useRef, useState } from "react";
import { Dialog, Modal, ModalOverlay } from "react-aria-components";
import { askQuestion } from "../lib/askAi";
import { fuzzyFilter } from "../lib/fuzzy";
import { kbd } from "../lib/platform";
import { useConnections } from "../state/connections";
import { useHistory } from "../state/history";
import { useSettings } from "../state/settings";
import { useTabs } from "../state/tabs";
import { useUi } from "../state/ui";
import { BookmarkIcon, CodeIcon, CommandIcon, DatabaseIcon, SettingsIcon, SparklesIcon, TableIcon, ViewIcon } from "./icons";
import s from "./CommandPalette.module.css";
import { exportConnections, importConnections, openSample } from "../lib/connectionFiles";
import { reportProblem } from "../lib/report";

interface Command {
  id: string;
  title: string;
  /** Shown faint after the title: a schema, a connection, a shortcut… */
  hint?: string;
  group: string;
  icon: React.ReactNode;
  run(): void;
}

/** ⌘K: jump to any table, connection or saved query, or run an app command, by typing. */
export function CommandPalette({ onNewConnection, onOpenSql }: { onNewConnection(): void; onOpenSql(sql: string): void }) {
  const open = useUi((st) => st.palette);
  const close = () => useUi.getState().setPalette(false);
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const list = useRef<HTMLDivElement>(null);

  const connections = useConnections((st) => st.connections);
  const activeId = useConnections((st) => st.activeId);
  const snapshot = useConnections((st) => (st.activeId ? st.live[st.activeId]?.schema : undefined));
  const saved = useHistory((st) => st.saved);
  const developerMode = useSettings((st) => st.developerMode);

  useEffect(() => {
    if (open) {
      setQuery("");
      setIndex(0);
    }
  }, [open]);

  const commands = useMemo<Command[]>(() => {
    const out: Command[] = [];
    if (activeId && snapshot) {
      const multi = snapshot.schemas.length > 1;
      for (const sc of snapshot.schemas)
        for (const t of sc.tables)
          out.push({
            id: `table:${sc.name}.${t.name}`,
            title: t.name,
            hint: multi ? sc.name : t.kind === "view" ? "view" : undefined,
            group: "Tables",
            icon: t.kind === "view" ? <ViewIcon size={15} /> : <TableIcon size={15} />,
            run: () => useTabs.getState().openTable(activeId, sc.name, t.name),
          });
    }
    for (const q of saved.filter((q) => !q.connectionId || q.connectionId === activeId))
      out.push({ id: `saved:${q.id}`, title: q.name, hint: "saved query", group: "Saved queries", icon: <BookmarkIcon size={15} />, run: () => onOpenSql(q.sql) });
    const ui = useUi.getState();
    const settings = useSettings.getState();
    out.push(
      { id: "new-query", title: "New SQL query", hint: kbd("T"), group: "Commands", icon: <CodeIcon size={15} />, run: () => activeId && useTabs.getState().open({ connectionId: activeId }) },
      { id: "new-connection", title: "New connection", hint: kbd("N"), group: "Commands", icon: <DatabaseIcon size={15} />, run: onNewConnection },
      { id: "diagram", title: "Show schema diagram", group: "Commands", icon: <CommandIcon size={15} />, run: () => ui.openTool("diagram") },
      { id: "objects", title: "Functions, triggers, sequences and users", group: "Commands", icon: <CommandIcon size={15} />, run: () => ui.openTool("objects") },
      { id: "migrate", title: "Move data from another database", group: "Commands", icon: <CommandIcon size={15} />, run: () => ui.openTool("migrate") },
      { id: "compare", title: "Compare with another database", group: "Commands", icon: <CommandIcon size={15} />, run: () => ui.openTool("compare") },
      { id: "backup", title: "Back up or restore", group: "Commands", icon: <CommandIcon size={15} />, run: () => ui.openTool("backup") },
      { id: "sample", title: "Open the sample database", hint: "recreated fresh", group: "Commands", icon: <DatabaseIcon size={15} />, run: () => openSample() },
      { id: "export-connections", title: "Export connections", group: "Commands", icon: <DatabaseIcon size={15} />, run: () => exportConnections() },
      { id: "import-connections", title: "Import connections", group: "Commands", icon: <DatabaseIcon size={15} />, run: () => importConnections() },
      { id: "report", title: "Report a problem", group: "Commands", icon: <SettingsIcon size={15} />, run: () => reportProblem("").catch(() => {}) },
      { id: "settings", title: "Settings", hint: kbd(","), group: "Commands", icon: <SettingsIcon size={15} />, run: () => ui.openSettings() },
      { id: "ai", title: "Set up AI", group: "Commands", icon: <SettingsIcon size={15} />, run: () => ui.openSettings("ai") },
      { id: "dev", title: developerMode ? "Turn off developer mode" : "Turn on developer mode", group: "Commands", icon: <SettingsIcon size={15} />, run: () => settings.set({ developerMode: !developerMode }) },
      { id: "theme", title: "Switch light / dark theme", group: "Commands", icon: <SettingsIcon size={15} />, run: () => settings.set({ theme: document.documentElement.dataset.theme === "light" || (!document.documentElement.dataset.theme && matchMedia("(prefers-color-scheme: light)").matches) ? "dark" : "light" }) },
      { id: "panel", title: "Show or hide the details panel", hint: kbd("I"), group: "Commands", icon: <SettingsIcon size={15} />, run: () => settings.set({ inspectorOpen: !settings.inspectorOpen }) },
    );
    for (const c of connections)
      out.push({
        id: `conn:${c.id}`,
        title: c.name,
        hint: c.id === activeId ? "connected" : c.env,
        group: "Connections",
        icon: <DatabaseIcon size={15} />,
        run: () => useConnections.getState().activate(c.id),
      });
    return out;
  }, [activeId, snapshot, saved, connections, developerMode, onNewConnection, onOpenSql]);

  const shown = useMemo(() => {
    const found = fuzzyFilter(commands, query, (c) => `${c.title} ${c.hint ?? ""}`).slice(0, 60);
    // Anything typed can also be asked as a question about the data.
    if (activeId && query.trim().length > 3) {
      const ask: Command = { id: "ask", title: `Ask AI: ${query.trim()}`, group: "Ask", icon: <SparklesIcon size={15} />, run: () => askQuestion(activeId, query) };
      return found.length && found[0].title.toLowerCase().includes(query.trim().toLowerCase()) ? [...found, ask] : [ask, ...found];
    }
    return found;
  }, [commands, query, activeId]);
  useEffect(() => setIndex(0), [query]);
  useEffect(() => {
    list.current?.querySelector<HTMLElement>(`[data-index="${index}"]`)?.scrollIntoView({ block: "nearest" });
  }, [index]);

  const choose = (c: Command | undefined) => {
    if (!c) return;
    close();
    c.run();
  };

  return (
    <ModalOverlay isOpen={open} onOpenChange={(o) => !o && close()} isDismissable className={s.overlay}>
      <Modal className={s.modal}>
        <Dialog className={s.dialog} aria-label="Command palette">
          <input
            className={s.input}
            autoFocus
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={activeId ? "Jump to a table, saved query or command…" : "Type a command…"}
            spellCheck={false}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown") setIndex((i) => Math.min(i + 1, shown.length - 1));
              else if (e.key === "ArrowUp") setIndex((i) => Math.max(i - 1, 0));
              else if (e.key === "Enter") choose(shown[index]);
              else return;
              e.preventDefault();
            }}
          />
          <div className={s.list} ref={list} role="listbox">
            {shown.map((c, i) => (
              <button
                key={c.id}
                role="option"
                aria-selected={i === index}
                data-index={i}
                className={s.item}
                onMouseMove={() => setIndex(i)}
                onClick={() => choose(c)}
              >
                <span className={s.icon}>{c.icon}</span>
                <span className={s.title}>{c.title}</span>
                {c.hint && <span className={s.hint}>{c.hint}</span>}
                {(i === 0 || shown[i - 1].group !== c.group) && <span className={s.group}>{c.group}</span>}
              </button>
            ))}
            {shown.length === 0 && <p className={s.empty}>Nothing matches “{query}”.</p>}
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
