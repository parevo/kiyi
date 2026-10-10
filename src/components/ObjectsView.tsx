import { useEffect, useMemo, useState } from "react";
import { HighlightedSql } from "../lib/highlight";
import { errorMessage, ipc } from "../lib/ipc";
import type { DbObject, ObjectKind, ObjectList, ObjectSource } from "../lib/types";
import { useActiveConnection, useConnections } from "../state/connections";
import { useTabs } from "../state/tabs";
import { toast } from "../state/toasts";
import { useUi } from "../state/ui";
import { AlertIcon, CloseIcon, CodeIcon, CopyIcon, ObjectsIcon, PlusIcon, RefreshIcon, SearchIcon, Spinner, TrashIcon } from "./icons";
import { Button, IconButton } from "./ui";
import s from "./ObjectsView.module.css";

const TITLES: Record<ObjectKind, string> = {
  function: "Functions",
  procedure: "Procedures",
  trigger: "Triggers",
  sequence: "Sequences",
  user: "Users & roles",
};

const SINGULAR: Record<ObjectKind, string> = {
  function: "function",
  procedure: "procedure",
  trigger: "trigger",
  sequence: "sequence",
  user: "user or role",
};

const NEW_LABEL: Record<ObjectKind, string> = {
  function: "New function",
  procedure: "New procedure",
  trigger: "New trigger",
  sequence: "New sequence",
  user: "New user",
};

/** What each kind is, for people who haven't met it. */
const ABOUT: Record<ObjectKind, string> = {
  function: "Reusable calculations that queries can call.",
  procedure: "Saved steps you run with CALL or EXEC.",
  trigger: "Code the database runs by itself when rows change.",
  sequence: "Counters that hand out the next number, often for IDs.",
  user: "Who can sign in, and what they're allowed to do.",
};

const keyOf = (o: DbObject) => `${o.kind}:${o.schema ?? ""}:${o.name}:${o.key}:${o.detail}`;

/**
 * Everything besides tables and views: functions, procedures, triggers, sequences, users.
 * Read here; changed through the SQL editor, where production connections get their usual review.
 */
export function ObjectsView() {
  const connection = useActiveConnection();
  const snapshot = useConnections((st) => (connection ? st.live[connection.id]?.schema : undefined));
  const close = () => useUi.getState().openTool(null);
  const [list, setList] = useState<ObjectList | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [needle, setNeedle] = useState("");
  const [selected, setSelected] = useState<DbObject | null>(null);
  const [source, setSource] = useState<ObjectSource | null>(null);
  const [sourceError, setSourceError] = useState<string | null>(null);

  const load = () => {
    if (!connection) return;
    setList(null);
    setError(null);
    ipc.listObjects(connection.id).then(setList, (e) => setError(errorMessage(e)));
  };
  useEffect(load, [connection]);

  useEffect(() => {
    setSource(null);
    setSourceError(null);
    if (!connection || !selected) return;
    let live = true;
    ipc.objectSource(connection.id, selected).then(
      (src) => live && setSource(src),
      (e) => live && setSourceError(errorMessage(e)),
    );
    return () => {
      live = false;
    };
  }, [connection, selected]);

  const groups = useMemo(() => {
    if (!list) return [];
    const q = needle.trim().toLowerCase();
    return list.kinds.map((kind) => ({
      kind,
      items: list.objects.filter((o) => o.kind === kind && (!q || `${o.schema ?? ""}.${o.name} ${o.detail}`.toLowerCase().includes(q))),
    }));
  }, [list, needle]);

  if (!connection) return null;
  const kind = connection.kind;
  const schemaCount = new Set(list?.objects.map((o) => o.schema).filter(Boolean)).size;
  const defaultSchema = snapshot?.defaultSchema ?? null;

  const openSql = (sql: string, title: string) => {
    useTabs.getState().open({ connectionId: connection.id, sql, title });
    close();
  };

  const startNew = async (k: ObjectKind) => {
    try {
      const sql = await ipc.objectTemplate(connection.id, k, k === "user" && kind !== "mysql" ? null : defaultSchema);
      openSql(sql, NEW_LABEL[k]);
    } catch (e) {
      toast.error(errorMessage(e));
    }
  };

  const qualified = (o: DbObject) => (o.schema && (schemaCount > 1 || o.schema !== defaultSchema) && kind !== "sqlite" ? `${o.schema}.${o.name}` : o.name);

  return (
    <div className={s.view}>
      <div className={s.toolbar}>
        <b>
          <ObjectsIcon size={15} /> Objects · {connection.name}
        </b>
        <label className={s.search}>
          <SearchIcon size={14} />
          <input value={needle} onChange={(e) => setNeedle(e.target.value)} placeholder="Find by name" spellCheck={false} aria-label="Find an object" />
        </label>
        <span className={s.spacer} />
        <IconButton label="Refresh" onPress={load}>
          <RefreshIcon />
        </IconButton>
        <IconButton label="Close" onPress={close}>
          <CloseIcon />
        </IconButton>
      </div>

      <div className={s.split}>
        <div className={s.list}>
          {error && (
            <p className={s.error} role="alert">
              <AlertIcon size={14} /> {error}
            </p>
          )}
          {!list && !error && (
            <p className={s.hint}>
              <Spinner size={13} /> Reading the catalog…
            </p>
          )}
          {list?.notes.map((n) => (
            <p key={n} className={s.note}>
              {n}
            </p>
          ))}
          {groups.map(({ kind: k, items }) => (
            <section key={k} className={s.group}>
              <div className={s.groupHead}>
                <span>
                  {TITLES[k]} <span className={s.count}>{items.length}</span>
                </span>
                <IconButton label={NEW_LABEL[k]} onPress={() => startNew(k)}>
                  <PlusIcon size={14} />
                </IconButton>
              </div>
              {items.length === 0 && <p className={s.empty}>{needle ? "No matches." : `None yet. ${ABOUT[k]}`}</p>}
              {items.map((o) => (
                <button key={keyOf(o)} className={s.item} data-selected={selected && keyOf(selected) === keyOf(o) ? true : undefined} onClick={() => setSelected(o)}>
                  <span className={s.itemName}>{qualified(o)}</span>
                  {o.detail && <span className={s.itemDetail}>{o.detail}</span>}
                </button>
              ))}
            </section>
          ))}
        </div>

        <div className={s.detail}>
          {!selected && (
            <div className={s.placeholder}>
              <ObjectsIcon size={28} />
              <p>Pick something on the left to see the SQL that defines it.</p>
              <p className={s.hint}>Changes go through the SQL editor, so you see exactly what runs{connection.env === "production" ? ", and production asks before anything changes" : ""}.</p>
            </div>
          )}
          {selected && (
            <>
              <div className={s.detailHead}>
                <div>
                  <h2>{qualified(selected)}</h2>
                  <p className={s.hint}>
                    {SINGULAR[selected.kind]}
                    {selected.detail && ` · ${selected.detail}`}
                  </p>
                </div>
                <div className={s.actions}>
                  <Button variant="primary" isDisabled={!source} onPress={() => source && openSql(source.definition, selected.name)}>
                    <CodeIcon size={14} /> Open in SQL editor
                  </Button>
                  <Button
                    isDisabled={!source}
                    onPress={() => source && navigator.clipboard.writeText(source.definition).then(() => toast.success("Copied"), () => toast.error("Couldn't copy"))}
                  >
                    <CopyIcon size={14} /> Copy
                  </Button>
                  <Button isDisabled={!source || connection.readOnly} onPress={() => source && openSql(`-- Review, then run to drop ${selected.name}.\n${source.drop}`, `Drop ${selected.name}`)}>
                    <TrashIcon size={14} /> Drop…
                  </Button>
                </div>
              </div>
              {sourceError && (
                <p className={s.error} role="alert">
                  <AlertIcon size={14} /> {sourceError}
                </p>
              )}
              {!source && !sourceError && (
                <p className={s.hint}>
                  <Spinner size={13} /> Loading…
                </p>
              )}
              {source && (
                <pre className={`${s.code} selectable`}>
                  <HighlightedSql sql={source.definition} kind={kind} />
                </pre>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
