import { useMemo, useState } from "react";
import { quoteIdent } from "../lib/dialect";
import { Dialog, DialogTrigger, Heading, Modal, ModalOverlay, Popover, Button as AriaButton } from "react-aria-components";
import { errorMessage, ipc } from "../lib/ipc";
import type { ConnectionConfig, TableAction, TableInfo } from "../lib/types";
import { connectionWhere, driverFor, useCatalog } from "../state/catalog";
import { useConnections } from "../state/connections";
import { useSettings } from "../state/settings";
import { tableKey, useTabs } from "../state/tabs";
import { useUi } from "../state/ui";
import { ContextMenu, type MenuState } from "./ContextMenu";
import { ChevronIcon, CodeIcon, HomeIcon, LockIcon, MoreIcon, PlusIcon, RefreshIcon, SearchIcon, SettingsIcon, Spinner, TableIcon, ViewIcon } from "./icons";
import { ReviewDialog, type ReviewRequest } from "./ReviewDialog";
import { Button, Field, IconButton } from "./ui";
import d from "./dialog.module.css";
import s from "./Sidebar.module.css";

export function Sidebar({
  onNewConnection,
  onEdit,
  onOpenSql,
}: {
  onNewConnection(): void;
  onEdit(c: ConnectionConfig): void;
  onOpenSql(sql: string): void;
}) {
  const activeId = useConnections((st) => st.activeId);
  const live = useConnections((st) => (activeId ? st.live[activeId] : undefined));
  const refreshSchema = useConnections((st) => st.refreshSchema);
  const activeTabId = useTabs((st) => st.activeId);
  const developerMode = useSettings((st) => st.developerMode);
  const hasConnections = useConnections((st) => st.connections.length > 0);

  return (
    <aside className={s.sidebar}>
      <div className={s.top} data-tauri-drag-region />
      {hasConnections && <ConnectionSwitcher onNew={onNewConnection} onEdit={onEdit} />}

      {live?.status === "error" && <div className={`${s.connError} selectable`}>{live.error}</div>}
      {live?.status === "connecting" && (
        <div className={s.connecting}>
          <Spinner size={12} /> Connecting…
        </div>
      )}

      {activeId && live?.status === "connected" && (
        <>
          <nav className={s.nav}>
            <button className={s.navItem} data-active={activeTabId === null || undefined} onClick={() => useTabs.setState({ activeId: null })}>
              <HomeIcon size={15} /> Overview
            </button>
          </nav>
          <TableList
            connectionId={activeId}
            loading={!!live.schemaLoading}
            onRefresh={() => refreshSchema(activeId)}
            onOpenSql={onOpenSql}
          />
        </>
      )}
      {!activeId && <SidebarEmpty onNew={onNewConnection} />}

      <div className={s.footer}>
        {activeId && live?.status === "connected" && (
          <button className={s.footerButton} onClick={() => onOpenSql("")}>
            <CodeIcon size={15} /> SQL editor
          </button>
        )}
        <span className={s.flex} />
        <SettingsButton />
      </div>
      {developerMode && live?.serverVersion && <div className={s.version}>{live.serverVersion}</div>}
    </aside>
  );
}

/** No connection open: say what this area is for, and offer the next step. */
function SidebarEmpty({ onNew }: { onNew(): void }) {
  const connections = useConnections((st) => st.connections);
  const activate = useConnections((st) => st.activate);
  return (
    <div className={s.emptySide}>
      {connections.length > 0 ? (
        <>
          <div className={s.section}>Connections</div>
          {connections.map((c) => (
            <button key={c.id} className={s.tableRow} onClick={() => activate(c.id)}>
              <span className={s.dot} style={{ "--env": `var(--env-${c.env})` } as React.CSSProperties} />
              <span className={s.tableName}>{c.name}</span>
            </button>
          ))}
        </>
      ) : (
        <p className={s.emptyHint}>Your databases and their tables will appear here once you connect.</p>
      )}
      <Button variant="ghost" onPress={onNew}>
        <PlusIcon size={14} /> New connection
      </Button>
    </div>
  );
}

function ConnectionSwitcher({ onNew, onEdit }: { onNew(): void; onEdit(c: ConnectionConfig): void }) {
  const connections = useConnections((st) => st.connections);
  const activeId = useConnections((st) => st.activeId);
  const live = useConnections((st) => st.live);
  const activate = useConnections((st) => st.activate);
  const disconnect = useConnections((st) => st.disconnect);
  const remove = useConnections((st) => st.remove);
  const drivers = useCatalog((st) => st.drivers);
  const [open, setOpen] = useState(false);
  const active = connections.find((c) => c.id === activeId);

  return (
    <DialogTrigger isOpen={open} onOpenChange={setOpen}>
      <AriaButton className={s.switcher} aria-label="Choose connection">
        {active ? (
          <>
            <span className={s.dot} data-on={live[active.id]?.status === "connected" || undefined} style={{ "--env": `var(--env-${active.env})` } as React.CSSProperties} />
            <span className={s.switcherText}>
              <span className={s.switcherName}>{active.name}</span>
              <span className={s.switcherMeta}>
                {driverFor(active, drivers)?.name}
                {active.readOnly && " · salt okunur"}
              </span>
            </span>
          </>
        ) : (
          <span className={s.switcherText}>
            <span className={s.switcherName}>{connections.length ? "Choose a connection" : "No connections"}</span>
          </span>
        )}
        <ChevronIcon size={12} className={s.switcherChevron} />
      </AriaButton>
      <Popover className={s.popover} placement="bottom start" offset={4}>
        <Dialog className={s.popDialog} aria-label="Connections">
          <div className={s.popList}>
            {connections.map((c) => {
              const status = live[c.id]?.status;
              return (
                <div key={c.id} className={s.popItem} data-active={c.id === activeId || undefined}>
                  <button
                    className={s.popMain}
                    onClick={() => {
                      setOpen(false);
                      activate(c.id);
                    }}
                  >
                    <span className={s.dot} data-on={status === "connected" || undefined} style={{ "--env": `var(--env-${c.env})` } as React.CSSProperties} />
                    <span className={s.switcherText}>
                      <span className={s.switcherName}>{c.name}</span>
                      <span className={s.switcherMeta}>
                        {driverFor(c, drivers)?.name} · {connectionWhere(c)}
                      </span>
                    </span>
                    {c.readOnly && <LockIcon size={12} />}
                    {status === "connecting" && <Spinner size={12} />}
                  </button>
                  <span className={s.popActions}>
                    <IconButton
                      label="Edit connection"
                      onPress={() => {
                        setOpen(false);
                        onEdit(c);
                      }}
                    >
                      <MoreIcon size={14} />
                    </IconButton>
                  </span>
                </div>
              );
            })}
          </div>
          <div className={s.popFooter}>
            <button
              className={s.popAction}
              onClick={() => {
                setOpen(false);
                onNew();
              }}
            >
              <PlusIcon size={14} /> New connection
            </button>
            {active && live[active.id]?.status === "connected" && (
              <button
                className={s.popAction}
                onClick={() => {
                  setOpen(false);
                  disconnect(active.id);
                }}
              >
                Disconnect
              </button>
            )}
            {active && (
              <button
                className={`${s.popAction} ${s.danger}`}
                onClick={() => {
                  setOpen(false);
                  if (confirm(`Remove the connection "${active.name}"? Its saved password is removed too. The database itself is not touched.`)) remove(active.id);
                }}
              >
                Remove this connection
              </button>
            )}
          </div>
        </Dialog>
      </Popover>
    </DialogTrigger>
  );
}

function SettingsButton() {
  const open = useUi((st) => st.openSettings);
  return (
    <AriaButton className={s.footerButton} aria-label="Settings" onPress={() => open()}>
      <SettingsIcon size={15} />
    </AriaButton>
  );
}

function TableList({
  connectionId,
  loading,
  onRefresh,
  onOpenSql,
}: {
  connectionId: string;
  loading: boolean;
  onRefresh(): void;
  onOpenSql(sql: string): void;
}) {
  const snapshot = useConnections((st) => st.live[connectionId]?.schema);
  const connection = useConnections((st) => st.connections.find((c) => c.id === connectionId))!;
  const developerMode = useSettings((st) => st.developerMode);
  const activeTabId = useTabs((st) => st.activeId);
  const activeKey = useTabs((st) => st.tabs.find((t) => t.id === activeTabId)?.table);
  const { openTable, openCreate, close, patch } = useTabs.getState();

  const schemas = snapshot?.schemas ?? [];
  const [schemaName, setSchemaName] = useState<string | null>(null);
  const current = schemas.find((x) => x.name === schemaName) ?? schemas.find((x) => x.name === snapshot?.defaultSchema) ?? schemas[0];
  const [filter, setFilter] = useState("");
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [review, setReview] = useState<ReviewRequest | null>(null);
  const [renaming, setRenaming] = useState<TableInfo | null>(null);

  const needle = filter.trim().toLowerCase();
  const { tables, views } = useMemo(() => {
    const list = (current?.tables ?? []).filter((t) => !needle || t.name.toLowerCase().includes(needle));
    return { tables: list.filter((t) => t.kind === "table"), views: list.filter((t) => t.kind === "view") };
  }, [current, needle]);

  const schema = current?.name ?? null;
  const q = (id: string) => quoteIdent(connection.kind, id);

  const runAction = async (table: TableInfo, action: TableAction) => {
    try {
      const statements = await ipc.planTableAction(connectionId, schema, table.name, table.kind === "view", action);
      const view = table.kind === "view";
      setReview({
        title: action.type === "drop" ? (view ? "Delete view" : "Delete table") : action.type === "truncate" ? "Empty table" : "Rename table",
        subtitle: table.name,
        summary:
          action.type === "drop"
            ? [{ text: view ? `The view ${table.name} will be deleted` : `The table ${table.name} and all of its rows will be permanently deleted`, danger: true }]
            : action.type === "truncate"
              ? [{ text: `Every row in ${table.name} will be deleted; its columns stay`, danger: true }]
              : [{ text: `${table.name} → ${action.to}` }],
        statements,
        action: action.type === "drop" ? "Delete permanently" : action.type === "truncate" ? "Delete all rows" : "Rename",
        confirmWord: table.name,
        run: async () => {
          await ipc.executeScript(connectionId, statements, "schema");
          const key = tableKey(connectionId, schema, table.name);
          const open = useTabs.getState().tabs.filter((t) => t.table === key || (t.connectionId === connectionId && t.schema === schema && t.tableName === table.name));
          if (action.type === "drop") open.forEach((t) => close(t.id, true));
          if (action.type === "rename") open.forEach((t) => patch(t.id, { table: tableKey(connectionId, schema, action.to), tableName: action.to, title: action.to }));
          onRefresh();
        },
      });
    } catch (e) {
      alert(errorMessage(e));
    }
  };

  const tableMenu = (x: number, y: number, t: TableInfo) => {
    const writable = !connection.readOnly;
    setMenu({
      x,
      y,
      items: [
        { label: "Open data", onSelect: () => openTable(connectionId, schema, t.name, "data") },
        { label: "Edit structure", onSelect: () => openTable(connectionId, schema, t.name, "structure") },
        ...(developerMode ? [{ label: "Query with SQL", onSelect: () => onOpenSql(`SELECT * FROM ${schema && schema !== snapshot?.defaultSchema ? `${q(schema)}.` : ""}${q(t.name)} LIMIT 100;`) }] : []),
        "separator",
        { label: "Rename…", onSelect: () => setRenaming(t), disabled: !writable },
        { label: "Delete all rows…", onSelect: () => runAction(t, { type: "truncate" }), disabled: !writable || t.kind === "view", danger: true },
        { label: t.kind === "view" ? "Delete view…" : "Delete table…", onSelect: () => runAction(t, { type: "drop" }), disabled: !writable, danger: true },
      ],
    });
  };

  const row = (t: TableInfo) => (
    <div
      key={t.name}
      className={s.tableRow}
      role="button"
      tabIndex={0}
      data-active={activeKey === tableKey(connectionId, schema, t.name) || undefined}
      onClick={() => openTable(connectionId, schema, t.name)}
      onKeyDown={(e) => e.key === "Enter" && openTable(connectionId, schema, t.name)}
      onContextMenu={(e) => {
        e.preventDefault();
        tableMenu(e.clientX, e.clientY, t);
      }}
    >
      {t.kind === "view" ? <ViewIcon size={14} /> : <TableIcon size={14} />}
      <span className={s.tableName}>
        <Highlight text={t.name} needle={needle} />
      </span>
      <button
        className={s.rowMenu}
        aria-label={`${t.name} options`}
        onClick={(e) => {
          e.stopPropagation();
          const r = e.currentTarget.getBoundingClientRect();
          tableMenu(r.left, r.bottom + 2, t);
        }}
      >
        <MoreIcon size={14} />
      </button>
    </div>
  );

  return (
    <>
      <label className={s.filter}>
        <SearchIcon size={14} />
        <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Find a table" spellCheck={false} />
      </label>
      {schemas.length > 1 && (
        <select className={s.schemaSelect} value={current?.name} onChange={(e) => setSchemaName(e.target.value)} aria-label="Schema">
          {schemas.map((x) => (
            <option key={x.name} value={x.name}>
              {x.name} ({x.tables.length})
            </option>
          ))}
        </select>
      )}

      <div className={s.tree}>
        <div className={s.section}>
          Tables
          <span className={s.sectionActions}>
            {!connection.readOnly && (
              <IconButton label="New table" onPress={() => openCreate(connectionId, schema)}>
                <PlusIcon size={14} />
              </IconButton>
            )}
            <IconButton label="Reload list" onPress={onRefresh} isDisabled={loading}>
              {loading ? <Spinner size={12} /> : <RefreshIcon size={13} />}
            </IconButton>
          </span>
        </div>
        {tables.map(row)}
        {!loading && tables.length === 0 && (
          <div className={s.empty}>
            {needle ? `No table matches "${filter}".` : "No tables yet."}
            {!needle && !connection.readOnly && (
              <Button variant="ghost" onPress={() => openCreate(connectionId, schema)}>
                <PlusIcon size={14} /> Create the first table
              </Button>
            )}
          </div>
        )}
        {views.length > 0 && (
          <>
            <div className={s.section}>Views</div>
            {views.map(row)}
          </>
        )}
      </div>

      <ContextMenu menu={menu} onClose={() => setMenu(null)} />
      <ReviewDialog request={review} kind={connection.kind} env={connection.env} onClose={() => setReview(null)} onOpenInEditor={onOpenSql} />
      {renaming && (
        <RenameDialog
          current={renaming.name}
          onClose={() => setRenaming(null)}
          onSubmit={(to) => {
            const t = renaming;
            setRenaming(null);
            runAction(t, { type: "rename", to });
          }}
        />
      )}
    </>
  );
}

function RenameDialog({ current, onClose, onSubmit }: { current: string; onClose(): void; onSubmit(to: string): void }) {
  const [value, setValue] = useState(current);
  const valid = value.trim() !== "" && value.trim() !== current;
  return (
    <ModalOverlay isOpen onOpenChange={(o) => !o && onClose()} isDismissable className={d.overlay}>
      <Modal className={`${d.modal} ${d.small}`}>
        <Dialog className={d.dialog}>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              if (valid) onSubmit(value.trim());
            }}
          >
            <div className={d.header}>
              <Heading slot="title" className={d.title}>
                Rename table
              </Heading>
            </div>
            <div className={d.body}>
              <Field label="New name" mono value={value} onChange={setValue} autoFocus />
            </div>
            <div className={d.footer}>
              <div className={d.footerRight}>
                <Button onPress={onClose}>Cancel</Button>
                <Button type="submit" variant="primary" isDisabled={!valid}>
                  Continue
                </Button>
              </div>
            </div>
          </form>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

function Highlight({ text, needle }: { text: string; needle: string }) {
  const i = needle ? text.toLowerCase().indexOf(needle) : -1;
  if (i < 0) return <>{text}</>;
  return (
    <>
      {text.slice(0, i)}
      <span className={s.match}>{text.slice(i, i + needle.length)}</span>
      {text.slice(i + needle.length)}
    </>
  );
}
