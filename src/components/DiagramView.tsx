import { useEffect, useMemo, useRef, useState } from "react";
import { type DiagramEdge, type DiagramTable, HEADER_H, layout, ROW_H } from "../lib/diagramLayout";
import { errorMessage, ipc } from "../lib/ipc";
import type { SchemaGraph } from "../lib/types";
import { useActiveConnection, useConnections } from "../state/connections";
import { useTabs } from "../state/tabs";
import { useUi } from "../state/ui";
import { CloseIcon, KeyIcon, LinkIcon, SearchIcon, Spinner } from "./icons";
import { IconButton } from "./ui";
import s from "./DiagramView.module.css";

/** Columns listed per table; keys always, then the first others. */
const MAX_COLUMNS = 12;
/** Without a search, show at most this many tables (the most connected first). */
const MAX_TABLES = 120;

/** The tables of the open connection and how they reference each other. */
export function DiagramView() {
  const connection = useActiveConnection();
  const snapshot = useConnections((st) => (connection ? st.live[connection.id]?.schema : undefined));
  const close = () => useUi.getState().openTool(null);
  const [graph, setGraph] = useState<SchemaGraph | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [needle, setNeedle] = useState("");
  const [schemaName, setSchemaName] = useState<string | null>(null);
  const [hover, setHover] = useState<string | null>(null);
  const [view, setView] = useState({ x: 40, y: 40, k: 1 });
  const host = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!connection) return;
    setGraph(null);
    ipc.schemaGraph(connection.id).then(setGraph, (e) => setError(errorMessage(e)));
  }, [connection]);

  const schemas = snapshot?.schemas ?? [];
  const current = schemaName ?? snapshot?.defaultSchema ?? schemas[0]?.name ?? null;
  const schema = schemas.find((x) => x.name === current);

  const { tables, edges, note } = useMemo(() => {
    if (!schema || !graph) return { tables: [] as DiagramTable[], edges: [] as DiagramEdge[], note: null as string | null };
    const rels = graph.relations.filter((r) => r.schema === schema.name && r.refSchema === schema.name);
    const pks = new Map(graph.primaryKeys.filter((p) => p.schema === schema.name).map((p) => [p.table, new Set(p.columns)]));
    const fkCols = new Map<string, Set<string>>();
    for (const r of rels) for (const c of r.columns) (fkCols.get(r.table) ?? fkCols.set(r.table, new Set()).get(r.table)!).add(c);
    const degree = new Map<string, number>();
    for (const r of rels) {
      degree.set(r.table, (degree.get(r.table) ?? 0) + 1);
      degree.set(r.refTable, (degree.get(r.refTable) ?? 0) + 1);
    }
    let picked = schema.tables;
    let note: string | null = null;
    const q = needle.trim().toLowerCase();
    if (q) {
      const hits = new Set(picked.filter((t) => t.name.toLowerCase().includes(q)).map((t) => t.name));
      // Show matches and the tables they connect to.
      for (const r of rels) {
        if (hits.has(r.table)) hits.add(r.refTable);
        else if (hits.has(r.refTable)) hits.add(r.table);
      }
      picked = picked.filter((t) => hits.has(t.name));
    } else if (picked.length > MAX_TABLES) {
      picked = [...picked].sort((a, b) => (degree.get(b.name) ?? 0) - (degree.get(a.name) ?? 0)).slice(0, MAX_TABLES);
      note = `Showing the ${MAX_TABLES} most connected of ${schema.tables.length} tables. Search to find others.`;
    }
    const tables: DiagramTable[] = picked.map((t) => {
      const pk = pks.get(t.name) ?? new Set<string>();
      const fk = fkCols.get(t.name) ?? new Set<string>();
      const keys = t.columns.filter((c) => pk.has(c.name) || fk.has(c.name));
      const rest = t.columns.filter((c) => !pk.has(c.name) && !fk.has(c.name));
      const shown = [...keys, ...rest.slice(0, Math.max(0, MAX_COLUMNS - keys.length))];
      return {
        id: t.name,
        name: t.name,
        columns: shown.map((c) => ({ name: c.name, type: c.dataType, pk: pk.has(c.name), fk: fk.has(c.name) })),
        hidden: t.columns.length - shown.length,
      };
    });
    const edges: DiagramEdge[] = rels.map((r) => ({ from: r.table, fromColumn: r.columns[0], to: r.refTable, toColumn: r.refColumns[0] ?? null }));
    return { tables, edges, note };
  }, [schema, graph, needle]);

  const placed = useMemo(() => layout(tables, edges), [tables, edges]);
  const byId = useMemo(() => new Map(placed.nodes.map((n) => [n.id, n])), [placed]);

  // Fit the drawing in the window when it changes.
  useEffect(() => {
    const el = host.current;
    if (!el || !placed.nodes.length) return;
    const { width, height } = el.getBoundingClientRect();
    const k = Math.min(1, (width - 80) / Math.max(1, placed.width), (height - 80) / Math.max(1, placed.height));
    setView({ k: Math.max(0.2, k), x: 40, y: 40 });
  }, [placed]);

  const rowY = (tableId: string, column: string | null) => {
    const n = byId.get(tableId)!;
    const i = column ? n.columns.findIndex((c) => c.name === column) : -1;
    return n.y + (i >= 0 ? HEADER_H + i * ROW_H + ROW_H / 2 + 4 : HEADER_H / 2);
  };

  const drag = useRef<{ x: number; y: number; vx: number; vy: number } | null>(null);
  const onWheel = (e: React.WheelEvent) => {
    if (e.ctrlKey || e.metaKey) {
      // Pinch or ⌘-scroll zooms around the pointer.
      const rect = host.current!.getBoundingClientRect();
      const px = e.clientX - rect.left;
      const py = e.clientY - rect.top;
      const k = Math.min(2, Math.max(0.15, view.k * Math.exp(-e.deltaY * 0.01)));
      setView({ k, x: px - ((px - view.x) / view.k) * k, y: py - ((py - view.y) / view.k) * k });
    } else setView({ ...view, x: view.x - e.deltaX, y: view.y - e.deltaY });
  };

  const openTable = (name: string) => {
    if (!connection) return;
    useTabs.getState().openTable(connection.id, current, name);
    close();
  };

  return (
    <div className={s.view}>
      <div className={s.toolbar}>
        <b>Schema diagram</b>
        {schemas.length > 1 && (
          <select value={current ?? ""} onChange={(e) => setSchemaName(e.target.value)} aria-label="Schema">
            {schemas.map((x) => (
              <option key={x.name}>{x.name}</option>
            ))}
          </select>
        )}
        <label className={s.search}>
          <SearchIcon size={13} />
          <input value={needle} onChange={(e) => setNeedle(e.target.value)} placeholder="Find a table" spellCheck={false} />
        </label>
        <span className={s.hint}>{note ?? "Drag to move · ⌘ + scroll or pinch to zoom · click a table to open it"}</span>
        <IconButton label="Close diagram" onPress={close}>
          <CloseIcon />
        </IconButton>
      </div>
      <div
        ref={host}
        className={s.canvas}
        onWheel={onWheel}
        onPointerDown={(e) => {
          if ((e.target as HTMLElement).closest("[data-table]")) return;
          drag.current = { x: e.clientX, y: e.clientY, vx: view.x, vy: view.y };
          (e.target as HTMLElement).setPointerCapture(e.pointerId);
        }}
        onPointerMove={(e) => drag.current && setView({ ...view, x: drag.current.vx + e.clientX - drag.current.x, y: drag.current.vy + e.clientY - drag.current.y })}
        onPointerUp={() => (drag.current = null)}
      >
        {error && <p className={s.message}>{error}</p>}
        {!error && !graph && (
          <p className={s.message}>
            <Spinner /> Reading relationships…
          </p>
        )}
        {graph && tables.length === 0 && <p className={s.message}>{needle ? `No table matches “${needle}”.` : "This schema has no tables yet."}</p>}
        {graph && tables.length > 0 && (
          <div className={s.world} style={{ transform: `translate(${view.x}px, ${view.y}px) scale(${view.k})` }}>
            <svg className={s.edges} width={placed.width + 200} height={placed.height + 200}>
              {edges.map((e, i) => {
                const a = byId.get(e.from);
                const b = byId.get(e.to);
                if (!a || !b) return null;
                const y1 = rowY(e.from, e.fromColumn);
                const y2 = rowY(e.to, e.toColumn);
                // From the referencing column to the referenced one, on the facing sides.
                const leftToRight = b.x + b.w <= a.x;
                const x1 = leftToRight ? a.x : a.x + a.w;
                const x2 = leftToRight ? b.x + b.w : b.x;
                const bend = Math.max(40, Math.abs(x1 - x2) / 2);
                const d = a === b ? `M${x1},${y1} c40,0 40,${y2 - y1} 0,${y2 - y1}` : `M${x1},${y1} C${x1 + (leftToRight ? -bend : bend)},${y1} ${x2 + (leftToRight ? bend : -bend)},${y2} ${x2},${y2}`;
                const lit = hover === e.from || hover === e.to;
                return <path key={i} d={d} className={s.edge} data-lit={lit || undefined} data-dim={(hover && !lit) || undefined} />;
              })}
            </svg>
            {placed.nodes.map((n) => (
              <div
                key={n.id}
                data-table
                className={s.table}
                data-dim={(hover && hover !== n.id && !edges.some((e) => (e.from === hover && e.to === n.id) || (e.to === hover && e.from === n.id))) || undefined}
                style={{ left: n.x, top: n.y, width: n.w }}
                onMouseEnter={() => setHover(n.id)}
                onMouseLeave={() => setHover(null)}
              >
                <button className={s.tableName} onClick={() => openTable(n.name)} title={`Open ${n.name}`}>
                  {n.name}
                </button>
                {n.columns.map((c) => (
                  <div key={c.name} className={s.column}>
                    <span className={s.icon}>{c.pk ? <KeyIcon size={11} /> : c.fk ? <LinkIcon size={11} /> : null}</span>
                    <span className={s.colName} data-key={c.pk || undefined}>
                      {c.name}
                    </span>
                    <span className={s.colType}>{c.type}</span>
                  </div>
                ))}
                {n.hidden > 0 && <div className={s.more}>+{n.hidden} more columns</div>}
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
