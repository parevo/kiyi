import { useMemo, useState } from "react";
import { Button as AriaButton, Dialog, Heading, Modal, ModalOverlay } from "react-aria-components";
import { errorMessage, ipc } from "../lib/ipc";
import type { ConnectionConfig, SchemaInfo, TableAction, TableInfo } from "../lib/types";
import { useConnections } from "../state/connections";
import { tableKey, useTabs } from "../state/tabs";
import { ContextMenu, type MenuState } from "./ContextMenu";
import { ChevronIcon, LockIcon, MoreIcon, PlusIcon, RefreshIcon, SearchIcon, Spinner, TableIcon, ViewIcon } from "./icons";
import { ReviewDialog, type ReviewRequest } from "./ReviewDialog";
import { ActionMenu, Button, Field, IconButton } from "./ui";
import d from "./dialog.module.css";
import s from "./Sidebar.module.css";
import ui from "./ui.module.css";

export function Sidebar({
  onNewConnection,
  onEdit,
  onOpenSql,
}: {
  onNewConnection(): void;
  onEdit(c: ConnectionConfig): void;
  onOpenSql(sql: string): void;
}) {
  const connections = useConnections((st) => st.connections);
  const activeId = useConnections((st) => st.activeId);
  const live = useConnections((st) => st.live);
  const activate = useConnections((st) => st.activate);
  const disconnect = useConnections((st) => st.disconnect);
  const remove = useConnections((st) => st.remove);
  const refreshSchema = useConnections((st) => st.refreshSchema);

  const active = activeId ? live[activeId] : undefined;

  return (
    <aside className={s.sidebar}>
      <div className={s.top} data-tauri-drag-region>
        <IconButton label="Yeni bağlantı" shortcut="⌘N" onPress={onNewConnection}>
          <PlusIcon />
        </IconButton>
      </div>

      <div className={s.section}>Bağlantılar</div>
      <div className={s.connections}>
        {connections.map((c) => {
          const state = live[c.id];
          return (
            <div
              key={c.id}
              role="button"
              tabIndex={0}
              className={s.conn}
              data-active={c.id === activeId || undefined}
              data-status={state?.status ?? "idle"}
              style={{ "--env": `var(--env-${c.env})` } as React.CSSProperties}
              onClick={() => activate(c.id)}
              onKeyDown={(e) => e.key === "Enter" && activate(c.id)}
              title={`${c.user}@${c.host}:${c.port}`}
            >
              <span className={s.dot} />
              <span className={s.connName}>{c.name}</span>
              <span className={s.connMeta} onClick={(e) => e.stopPropagation()}>
                {state?.status === "connecting" && <Spinner size={12} />}
                {c.readOnly && <LockIcon size={12} aria-label="Salt okunur" />}
                <span className={s.connMenu}>
                  <ActionMenu
                    trigger={
                      <AriaButton aria-label="Bağlantı seçenekleri" className={`${ui.button} ${ui.ghost} ${ui.icon}`}>
                        <MoreIcon />
                      </AriaButton>
                    }
                    actions={[
                      { id: "edit", label: "Düzenle…", onAction: () => onEdit(c) },
                      ...(state?.status === "connected"
                        ? [{ id: "disconnect", label: "Bağlantıyı kes", onAction: () => disconnect(c.id) }]
                        : []),
                      {
                        id: "delete",
                        label: "Sil",
                        danger: true,
                        onAction: () => {
                          if (confirm(`"${c.name}" silinsin mi? Kayıtlı şifresi de keychain'den kaldırılır.`)) remove(c.id);
                        },
                      },
                    ]}
                  />
                </span>
              </span>
            </div>
          );
        })}
        {connections.length === 0 && <div className={s.empty}>Henüz bağlantı yok.</div>}
      </div>

      {active?.status === "error" && <div className={`${s.connError} selectable`}>{active.error}</div>}

      {activeId && active?.status === "connected" && (
        <>
          <div className={s.divider} />
          <SchemaTree
            connectionId={activeId}
            schemas={active.schema?.schemas ?? []}
            defaultSchema={active.schema?.defaultSchema ?? null}
            loading={!!active.schemaLoading}
            onRefresh={() => refreshSchema(activeId)}
            onOpenSql={onOpenSql}
          />
          <div className={s.footer} title={active.serverVersion}>
            {active.serverVersion}
          </div>
        </>
      )}
    </aside>
  );
}

function SchemaTree({
  connectionId,
  schemas,
  defaultSchema,
  loading,
  onRefresh,
  onOpenSql,
}: {
  connectionId: string;
  schemas: SchemaInfo[];
  defaultSchema: string | null;
  loading: boolean;
  onRefresh(): void;
  onOpenSql(sql: string): void;
}) {
  const [filter, setFilter] = useState("");
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [review, setReview] = useState<ReviewRequest | null>(null);
  const [renaming, setRenaming] = useState<{ schema: string; table: TableInfo } | null>(null);
  const tabs = useTabs((st) => st.tabs);
  const activeTabId = useTabs((st) => st.activeId);
  const { openTable, openCreate, close, patch } = useTabs.getState();
  const connection = useConnections((st) => st.connections.find((c) => c.id === connectionId))!;

  const needle = filter.trim().toLowerCase();
  const visible = useMemo(
    () =>
      schemas
        .map((sc) => ({ ...sc, tables: needle ? sc.tables.filter((t) => t.name.toLowerCase().includes(needle)) : sc.tables }))
        .filter((sc) => !needle || sc.tables.length > 0),
    [schemas, needle],
  );

  const isOpen = (name: string) => (needle ? true : (expanded[name] ?? (name === defaultSchema || schemas.length === 1)));
  const activeKey = tabs.find((t) => t.id === activeTabId)?.table;

  const q = (id: string) => (connection.kind === "mysql" ? `\`${id.replace(/`/g, "``")}\`` : `"${id.replace(/"/g, '""')}"`);
  const qualified = (schema: string, table: string) => (schema === defaultSchema ? q(table) : `${q(schema)}.${q(table)}`);

  const runAction = async (schema: string, table: TableInfo, action: TableAction) => {
    try {
      const statements = await ipc.planTableAction(connectionId, schema, table.name, table.kind === "view", action);
      const verb = action.type === "drop" ? "sil" : action.type === "truncate" ? "boşalt" : "yeniden adlandır";
      setReview({
        title: `${table.name} tablosunu ${verb}`,
        subtitle:
          action.type === "drop"
            ? "Tablo ve içindeki tüm veri kalıcı olarak silinir."
            : action.type === "truncate"
              ? "Tablodaki tüm satırlar silinir, yapı kalır."
              : undefined,
        statements,
        action: action.type === "drop" ? "Kalıcı olarak sil" : action.type === "truncate" ? "Tüm satırları sil" : "Yeniden adlandır",
        confirmWord: table.name,
        run: async () => {
          await ipc.executeScript(connectionId, statements, "schema");
          const key = tableKey(connectionId, schema, table.name);
          const open = useTabs.getState().tabs.filter((t) => t.table === key || (t.tableName === table.name && t.schema === schema && t.connectionId === connectionId));
          if (action.type === "drop") open.forEach((t) => close(t.id, true));
          if (action.type === "rename") {
            open.forEach((t) => patch(t.id, { table: tableKey(connectionId, schema, action.to), tableName: action.to, title: action.to }));
          }
          onRefresh();
        },
      });
    } catch (e) {
      alert(errorMessage(e));
    }
  };

  const tableMenu = (x: number, y: number, schema: string, t: TableInfo) => {
    const writable = !connection.readOnly;
    setMenu({
      x,
      y,
      items: [
        { label: "Veriyi aç", onSelect: () => openTable(connectionId, schema, t.name, "data") },
        { label: "Yapıyı aç", onSelect: () => openTable(connectionId, schema, t.name, "structure") },
        { label: "SELECT sorgusu aç", onSelect: () => onOpenSql(`SELECT * FROM ${qualified(schema, t.name)} LIMIT 100;`) },
        "separator",
        { label: "Yeniden adlandır…", onSelect: () => setRenaming({ schema, table: t }), disabled: !writable },
        { label: "Boşalt (TRUNCATE)…", onSelect: () => runAction(schema, t, { type: "truncate" }), disabled: !writable || t.kind === "view", danger: true },
        { label: t.kind === "view" ? "View'ı sil…" : "Tabloyu sil…", onSelect: () => runAction(schema, t, { type: "drop" }), disabled: !writable, danger: true },
        "separator",
        { label: "Adı kopyala", onSelect: () => navigator.clipboard.writeText(t.name) },
      ],
    });
  };

  return (
    <>
      <div className={s.section}>
        Şema
        <span className={s.sectionActions}>
          {!connection.readOnly && (
            <IconButton label="Yeni tablo" onPress={() => openCreate(connectionId, defaultSchema ?? schemas[0]?.name ?? null)}>
              <PlusIcon size={14} />
            </IconButton>
          )}
          <IconButton label="Şemayı yenile" onPress={onRefresh} isDisabled={loading}>
            {loading ? <Spinner size={12} /> : <RefreshIcon size={14} />}
          </IconButton>
        </span>
      </div>
      <label className={s.filter}>
        <SearchIcon size={14} />
        <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Tablo ara" spellCheck={false} />
      </label>
      <div className={s.tree}>
        {visible.map((sc) => (
          <div key={sc.name}>
            {schemas.length > 1 && (
              <AriaButton
                className={s.schemaRow}
                aria-expanded={isOpen(sc.name)}
                onPress={() => setExpanded((e) => ({ ...e, [sc.name]: !isOpen(sc.name) }))}
              >
                <ChevronIcon size={12} className={s.chevron} />
                {sc.name}
                <span className={s.count}>{sc.tables.length}</span>
              </AriaButton>
            )}
            {isOpen(sc.name) &&
              sc.tables.map((t) => (
                <div
                  key={t.name}
                  className={s.tableRow}
                  role="button"
                  tabIndex={0}
                  style={schemas.length > 1 ? undefined : { paddingLeft: 8 }}
                  data-active={activeKey === tableKey(connectionId, sc.name, t.name) || undefined}
                  onClick={() => openTable(connectionId, sc.name, t.name)}
                  onKeyDown={(e) => e.key === "Enter" && openTable(connectionId, sc.name, t.name)}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    tableMenu(e.clientX, e.clientY, sc.name, t);
                  }}
                >
                  {t.kind === "view" ? <ViewIcon size={14} /> : <TableIcon size={14} />}
                  <span className={s.tableName}>
                    <Highlight text={t.name} needle={needle} />
                  </span>
                  <button
                    className={s.rowMenu}
                    aria-label={`${t.name} seçenekleri`}
                    onClick={(e) => {
                      e.stopPropagation();
                      const r = e.currentTarget.getBoundingClientRect();
                      tableMenu(r.left, r.bottom + 2, sc.name, t);
                    }}
                  >
                    <MoreIcon size={14} />
                  </button>
                </div>
              ))}
          </div>
        ))}
        {!loading && visible.length === 0 && (
          <div className={s.empty}>{needle ? `"${filter}" ile eşleşen tablo yok.` : "Bu veritabanında tablo yok."}</div>
        )}
      </div>

      <ContextMenu menu={menu} onClose={() => setMenu(null)} />
      <ReviewDialog request={review} kind={connection.kind} env={connection.env} onClose={() => setReview(null)} onOpenInEditor={onOpenSql} />
      {renaming && (
        <RenameDialog
          current={renaming.table.name}
          onClose={() => setRenaming(null)}
          onSubmit={(to) => {
            const r = renaming;
            setRenaming(null);
            runAction(r.schema, r.table, { type: "rename", to });
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
                Yeniden adlandır
              </Heading>
            </div>
            <div className={d.body}>
              <Field label="Yeni ad" mono value={value} onChange={setValue} autoFocus />
            </div>
            <div className={d.footer}>
              <div className={d.footerRight}>
                <Button onPress={onClose}>Vazgeç</Button>
                <Button type="submit" variant="primary" isDisabled={!valid}>
                  Devam
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
