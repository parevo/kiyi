import "@glideapps/glide-data-grid/dist/index.css";
import {
  CompactSelection,
  DataEditor,
  type DataEditorRef,
  type EditableGridCell,
  type GridCell,
  GridCellKind,
  type GridColumn,
  GridColumnIcon,
  type GridSelection,
  type Item,
  type Rectangle,
  type Theme,
} from "@glideapps/glide-data-grid";
import { forwardRef, useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import { errorMessage, ipc } from "../lib/ipc";
import type { BrowseRequest, Cell, ChangeSet, ColumnMeta, ConnectionConfig, ErrorInfo, Filter, RowChange, TableDetails, ValueKind } from "../lib/types";
import { useTabs } from "../state/tabs";
import { ContextMenu, type MenuEntry, type MenuState } from "./ContextMenu";
import { FilterBar, type FilterState } from "./FilterBar";
import { useGridTheme } from "./gridTheme";
import type { ReviewRequest } from "./ReviewDialog";
import s from "./TableView.module.css";

const PAGE_SIZE = 300;
const EXACT_COUNT_LIMIT = 5_000_000;

const ICONS: Record<ValueKind, GridColumnIcon> = {
  text: GridColumnIcon.HeaderString,
  number: GridColumnIcon.HeaderNumber,
  bool: GridColumnIcon.HeaderBoolean,
  json: GridColumnIcon.HeaderCode,
  temporal: GridColumnIcon.HeaderDate,
  uuid: GridColumnIcon.HeaderRowID,
  binary: GridColumnIcon.HeaderImage,
  array: GridColumnIcon.HeaderArray,
  other: GridColumnIcon.HeaderString,
};

/** Pending edits. `undefined` in an inserted row means "use the column default". */
interface Edits {
  updates: Map<number, Map<number, Cell>>;
  inserts: (Cell | undefined)[][];
  deletes: Set<number>;
}

const emptyEdits = (): Edits => ({ updates: new Map(), inserts: [], deletes: new Set() });
const cloneEdits = (e: Edits): Edits => ({
  updates: new Map([...e.updates].map(([r, m]) => [r, new Map(m)])),
  inserts: e.inserts.map((r) => [...r]),
  deletes: new Set(e.deletes),
});

export function countEdits(e: Edits) {
  let n = e.inserts.length + e.deletes.size;
  for (const [row] of e.updates) if (!e.deletes.has(row)) n++;
  return n;
}

export interface TableDataHandle {
  save(): void;
  undo(): void;
  discard(): void;
  refresh(): void;
  addRow(): void;
  deleteSelected(): void;
  toggleFilters(): void;
}

export interface TableDataStatus {
  loaded: number;
  total: number | null;
  totalIsEstimate: boolean;
  pending: number;
  selectedRows: number;
  loading: boolean;
  filters: number;
  showFilters: boolean;
  elapsedMs: number | null;
  sql: string | null;
}

interface Props {
  tabId: string;
  connection: ConnectionConfig;
  details: TableDetails;
  active: boolean;
  onStatus(status: TableDataStatus): void;
  onReview(req: ReviewRequest): void;
}

function preview(value: string) {
  const flat = value.length > 400 ? value.slice(0, 400) + "…" : value;
  return flat.replace(/\r?\n/g, " ⏎ ");
}

export const TableData = forwardRef<TableDataHandle, Props>(function TableData(
  { tabId, connection, details, active, onStatus, onReview },
  ref,
) {
  const theme = useGridTheme();
  const grid = useRef<DataEditorRef>(null);
  const tab = useTabs((st) => st.tabs.find((t) => t.id === tabId));
  const openTable = useTabs((st) => st.openTable);

  const design = details.design;
  const schema = details.schema;
  const pk = useMemo(() => design.columns.filter((c) => c.primaryKey).map((c) => c.name), [design]);
  const editable = !details.isView && !connection.readOnly && pk.length > 0;

  const [filterState, setFilterState] = useState<FilterState>({ filters: tab?.initialFilters ?? [], rawWhere: null });
  const [showFilters, setShowFilters] = useState(!!tab?.initialFilters?.length);
  const [sort, setSort] = useState<{ column: string; descending: boolean } | null>(null);

  const [columns, setColumns] = useState<ColumnMeta[]>([]);
  const rows = useRef<Cell[][]>([]);
  const [rowCount, setRowCount] = useState(0);
  const [exhausted, setExhausted] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<ErrorInfo | null>(null);
  const [total, setTotal] = useState<{ n: number; estimate: boolean } | null>(null);
  const [lastSql, setLastSql] = useState<string | null>(null);
  const [elapsed, setElapsed] = useState<number | null>(null);
  const generation = useRef(0);

  const edits = useRef<Edits>(emptyEdits());
  const history = useRef<Edits[]>([]);
  const [version, setVersion] = useState(0);
  const [selection, setSelection] = useState<GridSelection>({
    columns: CompactSelection.empty(),
    rows: CompactSelection.empty(),
  });
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [widths, setWidths] = useState<Record<string, number>>({});

  const designByName = useMemo(() => new Map(design.columns.map((c) => [c.name, c])), [design]);
  const fkByColumn = useMemo(() => {
    const m = new Map<string, (typeof design.foreignKeys)[number]>();
    for (const fk of design.foreignKeys) if (fk.columns.length === 1) m.set(fk.columns[0], fk);
    return m;
  }, [design]);

  const request = useCallback(
    (offset: number): BrowseRequest => ({
      schema,
      table: design.name,
      filters: filterState.filters,
      rawWhere: filterState.rawWhere,
      sort: sort ? [sort] : [],
      tiebreak: pk,
      limit: PAGE_SIZE,
      offset,
    }),
    [schema, design.name, filterState, sort, pk],
  );

  const pending = countEdits(edits.current);

  // ---- loading

  const load = useCallback(
    async (reset: boolean) => {
      const gen = reset ? ++generation.current : generation.current;
      const offset = reset ? 0 : rows.current.length;
      setLoading(true);
      if (reset) setError(null);
      const started = performance.now();
      try {
        const page = await ipc.browseTable(connection.id, request(offset));
        if (gen !== generation.current) return;
        if (reset) {
          rows.current = page.rows;
          setColumns(page.columns);
          setLastSql(page.sql);
        } else {
          rows.current.push(...page.rows);
        }
        setRowCount(rows.current.length);
        setExhausted(page.rows.length < PAGE_SIZE);
        setElapsed(performance.now() - started);
      } catch (e) {
        if (gen === generation.current) setError(typeof e === "object" ? (e as ErrorInfo) : { message: errorMessage(e), code: null, position: null });
      } finally {
        if (gen === generation.current) setLoading(false);
      }
    },
    [connection.id, request],
  );

  const loadCount = useCallback(async () => {
    const gen = generation.current;
    const unfiltered = !filterState.filters.length && !filterState.rawWhere;
    if (unfiltered && details.rowEstimate !== null && details.rowEstimate > EXACT_COUNT_LIMIT) {
      setTotal({ n: details.rowEstimate, estimate: true });
      return;
    }
    setTotal(null);
    try {
      const n = await ipc.countRows(connection.id, request(0));
      if (gen === generation.current) setTotal({ n, estimate: false });
    } catch {
      /* the page itself reports errors */
    }
  }, [connection.id, request, filterState, details.rowEstimate]);

  const reload = useCallback(() => {
    load(true);
    loadCount();
  }, [load, loadCount]);

  // Reload whenever the query shape changes. Edits are tied to row positions, so they go too.
  useEffect(() => {
    edits.current = emptyEdits();
    history.current = [];
    setVersion((v) => v + 1);
    reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [filterState, sort, design]);

  const guardPending = (action: string) =>
    countEdits(edits.current) === 0 || confirm(`Kaydedilmemiş ${countEdits(edits.current)} değişiklik var. ${action} için bunlar atılacak. Devam edilsin mi?`);

  // ---- edits

  const mutate = (fn: (e: Edits) => void) => {
    history.current.push(cloneEdits(edits.current));
    if (history.current.length > 200) history.current.shift();
    fn(edits.current);
    setVersion((v) => v + 1);
  };

  const baseRows = rowCount;
  const totalRows = baseRows + edits.current.inserts.length;

  const valueAt = (col: number, row: number): Cell | undefined => {
    if (row >= baseRows) return edits.current.inserts[row - baseRows]?.[col];
    const u = edits.current.updates.get(row);
    if (u?.has(col)) return u.get(col)!;
    return rows.current[row]?.[col] ?? null;
  };

  const setValue = (col: number, row: number, value: Cell) =>
    mutate((e) => {
      if (row >= baseRows) {
        e.inserts[row - baseRows][col] = value;
        return;
      }
      const original = rows.current[row]?.[col] ?? null;
      const u = e.updates.get(row) ?? new Map<number, Cell>();
      if (value === original) u.delete(col);
      else u.set(col, value);
      if (u.size) e.updates.set(row, u);
      else e.updates.delete(row);
    });

  const columnWritable = (col: number) => {
    const d = designByName.get(columns[col]?.name ?? "");
    return editable && !!d && !d.generated;
  };

  const addRow = () => {
    if (!editable) return;
    mutate((e) => e.inserts.push(columns.map(() => undefined)));
    requestAnimationFrame(() => grid.current?.scrollTo(0, baseRows + edits.current.inserts.length - 1));
  };

  const duplicateRows = (indices: number[]) =>
    mutate((e) => {
      for (const r of indices) {
        e.inserts.push(
          columns.map((c, i) => {
            const d = designByName.get(c.name);
            if (!d || d.autoIncrement || d.generated || (d.primaryKey && d.default)) return undefined;
            return valueAt(i, r);
          }),
        );
      }
    });

  const deleteRows = (indices: number[]) =>
    mutate((e) => {
      const existing = indices.filter((r) => r < baseRows);
      const inserted = new Set(indices.filter((r) => r >= baseRows).map((r) => r - baseRows));
      const allDeleted = existing.length > 0 && existing.every((r) => e.deletes.has(r));
      for (const r of existing) allDeleted ? e.deletes.delete(r) : e.deletes.add(r);
      e.inserts = e.inserts.filter((_, i) => !inserted.has(i));
    });

  const selectedRows = (): number[] => {
    const out = selection.rows.toArray();
    if (out.length) return out;
    if (selection.current) {
      const { y, height } = selection.current.range;
      return Array.from({ length: height }, (_, i) => y + i);
    }
    return [];
  };

  const undo = () => {
    const previous = history.current.pop();
    if (!previous) return;
    edits.current = previous;
    setVersion((v) => v + 1);
  };

  const discard = () => {
    if (!countEdits(edits.current)) return;
    history.current.push(cloneEdits(edits.current));
    edits.current = emptyEdits();
    setVersion((v) => v + 1);
  };

  // ---- save

  const save = async () => {
    const e = edits.current;
    if (!countEdits(e)) return;
    const index = (name: string) => columns.findIndex((c) => c.name === name);
    const key = (row: number) => pk.map((k) => ({ column: k, value: rows.current[row]?.[index(k)] ?? null }));
    const changes: RowChange[] = [];
    for (const r of [...e.deletes].sort((a, b) => a - b)) changes.push({ type: "delete", key: key(r) });
    for (const [r, cols] of [...e.updates].sort((a, b) => a[0] - b[0])) {
      if (e.deletes.has(r)) continue;
      changes.push({ type: "update", key: key(r), values: [...cols].map(([c, value]) => ({ column: columns[c].name, value })) });
    }
    for (const row of e.inserts) {
      const values = row.flatMap((value, c) => (value === undefined ? [] : [{ column: columns[c].name, value }]));
      changes.push({ type: "insert", values });
    }
    const set: ChangeSet = {
      schema,
      table: design.name,
      binaryColumns: columns.filter((c) => c.kind === "binary").map((c) => c.name),
      changes,
    };
    try {
      const statements = await ipc.planRowChanges(connection.id, set);
      onReview({
        title: "Değişiklikleri kaydet",
        subtitle: `${design.name} tablosunda ${changes.length} değişiklik, tek transaction içinde`,
        statements,
        action: "Kaydet",
        confirmWord: design.name,
        run: async () => {
          await ipc.executeScript(connection.id, statements, "data");
          edits.current = emptyEdits();
          history.current = [];
          setVersion((v) => v + 1);
          reload();
        },
      });
    } catch (err) {
      alert(errorMessage(err));
    }
  };

  useImperativeHandle(ref, () => ({
    save,
    undo,
    discard,
    refresh: () => guardPending("Yenilemek") && (discard(), reload()),
    addRow,
    deleteSelected: () => deleteRows(selectedRows()),
    toggleFilters: () => setShowFilters((v) => !v),
  }));

  // Status for the toolbar / footer, which live in the parent.
  const selectedCount = selectedRows().length;
  useEffect(() => {
    onStatus({
      loaded: baseRows,
      total: total?.n ?? null,
      totalIsEstimate: total?.estimate ?? false,
      pending,
      selectedRows: selectedCount,
      loading,
      filters: filterState.filters.length + (filterState.rawWhere ? 1 : 0),
      showFilters,
      elapsedMs: elapsed,
      sql: lastSql,
    });
  });

  // ---- keyboard (only for the visible tab)

  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent) => {
      if (!e.metaKey) return;
      const inText = (e.target as HTMLElement).closest("input, textarea, [contenteditable]");
      const k = e.key.toLowerCase();
      if (k === "s") {
        e.preventDefault();
        save();
      } else if (k === "z" && !e.shiftKey && !inText) {
        e.preventDefault();
        undo();
      } else if (k === "r") {
        e.preventDefault();
        if (guardPending("Yenilemek")) {
          discard();
          reload();
        }
      } else if (k === "f") {
        e.preventDefault();
        setShowFilters((v) => !v);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  // ---- grid

  const css = useMemo(() => {
    const v = (n: string) => getComputedStyle(document.documentElement).getPropertyValue(n).trim();
    return { edit: v("--edit-bg"), insert: v("--insert-bg"), del: v("--delete-bg"), nul: v("--null"), faint: v("--text-faint") };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [theme]);

  const gridColumns = useMemo<GridColumn[]>(
    () =>
      columns.map((c, i) => {
        const d = designByName.get(c.name);
        const arrow = sort?.column === c.name ? (sort.descending ? " ↓" : " ↑") : "";
        const keyMark = d?.primaryKey ? " 🔑" : fkByColumn.has(c.name) ? " ↗" : "";
        let width = widths[c.name];
        if (!width) {
          let longest = c.name.length + 5;
          for (let r = 0; r < Math.min(rows.current.length, 60); r++) {
            const v = rows.current[r][i];
            longest = Math.max(longest, v === null ? 4 : Math.min(v.length, 60));
          }
          width = Math.round(Math.min(380, Math.max(80, longest * 7.3 + 30)));
        }
        return { id: c.name, title: c.name + keyMark + arrow, icon: ICONS[c.kind], width };
      }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [columns, sort, widths, designByName, fkByColumn, rowCount > 0],
  );

  const getCellContent = useCallback(
    ([col, row]: Item): GridCell => {
      const meta = columns[col];
      const inserted = row >= baseRows;
      const deleted = !inserted && edits.current.deletes.has(row);
      const edited = !inserted && !!edits.current.updates.get(row)?.has(col);
      const value = valueAt(col, row);
      const writable = columnWritable(col) && !deleted;

      const bg = deleted ? css.del : inserted ? css.insert : edited ? css.edit : undefined;
      const over: Partial<Theme> = bg ? { bgCell: bg } : {};
      if (deleted) over.textDark = css.faint;

      if (value === undefined) {
        const d = designByName.get(meta?.name ?? "");
        const label = d?.autoIncrement || d?.default || d?.generated ? "DEFAULT" : "NULL";
        return {
          kind: GridCellKind.Text,
          data: "",
          displayData: label,
          allowOverlay: writable,
          readonly: !writable,
          themeOverride: { ...over, textDark: css.faint, baseFontStyle: "italic 12px" },
        };
      }
      if (meta?.kind === "bool") {
        return {
          kind: GridCellKind.Boolean,
          data: value === null ? null : value === "true",
          allowOverlay: false,
          readonly: !writable,
          copyData: value ?? "NULL",
          themeOverride: over,
        };
      }
      if (value === null) {
        return {
          kind: GridCellKind.Text,
          data: "",
          displayData: "NULL",
          allowOverlay: writable,
          readonly: !writable,
          copyData: "NULL",
          themeOverride: { ...over, textDark: deleted ? css.faint : css.nul, baseFontStyle: "italic 12px" },
        };
      }
      return {
        kind: GridCellKind.Text,
        data: value,
        displayData: preview(value),
        allowOverlay: true,
        readonly: !writable,
        copyData: value,
        contentAlign: meta?.kind === "number" ? "right" : undefined,
        themeOverride: over,
      };
    },
    // `version` captures edit changes; the ref contents are read at draw time.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [columns, baseRows, version, css, editable],
  );

  const onCellEdited = ([col, row]: Item, cell: EditableGridCell) => {
    if (cell.kind === GridCellKind.Boolean) {
      setValue(col, row, cell.data === null || cell.data === undefined ? null : String(cell.data));
    } else if (cell.kind === GridCellKind.Text) {
      const before = valueAt(col, row);
      // Opening a NULL/DEFAULT cell and closing it without typing isn't an edit.
      if (cell.data === "" && (before === null || before === undefined)) return;
      setValue(col, row, cell.data);
    }
  };

  const setNull = (cells: Item[]) => {
    const targets = cells.filter(([c, r]) => columnWritable(c) && designByName.get(columns[c].name)?.nullable && !edits.current.deletes.has(r));
    if (!targets.length) return;
    mutate((e) => {
      for (const [col, row] of targets) {
        if (row >= baseRows) e.inserts[row - baseRows][col] = null;
        else {
          const u = e.updates.get(row) ?? new Map<number, Cell>();
          if ((rows.current[row]?.[col] ?? null) === null) u.delete(col);
          else u.set(col, null);
          if (u.size) e.updates.set(row, u);
          else e.updates.delete(row);
        }
      }
    });
  };

  const rowJson = (row: number) =>
    JSON.stringify(Object.fromEntries(columns.map((c, i) => [c.name, valueAt(i, row) ?? null])), null, 2);

  const filterBy = (column: string, value: Cell) => {
    if (!guardPending("Filtrelemek")) return;
    const f: Filter = value === null ? { column, op: "isNull", value: "" } : { column, op: "eq", value };
    setShowFilters(true);
    setFilterState((st) => ({ ...st, filters: [...st.filters.filter((x) => x.column !== column), f] }));
  };

  const followFk = (column: string, value: Cell) => {
    const fk = fkByColumn.get(column);
    if (!fk || value === null) return;
    openTable(connection.id, fk.refSchema ?? schema, fk.refTable, "data", [{ column: fk.refColumns[0], op: "eq", value }]);
  };

  const onCellContextMenu = ([col, row]: Item, e: { bounds: Rectangle; localEventX: number; localEventY: number; preventDefault(): void }) => {
    e.preventDefault();
    if (row < 0 || col < 0) return;
    const meta = columns[col];
    const value = valueAt(col, row) ?? null;
    const rowsSel = selectedRows().includes(row) ? selectedRows() : [row];
    const fk = fkByColumn.get(meta.name);
    const nullable = designByName.get(meta.name)?.nullable;
    const items: MenuEntry[] = [
      { label: "Değeri kopyala", onSelect: () => navigator.clipboard.writeText(value ?? "NULL"), shortcut: "⌘C" },
      { label: "Satırı JSON olarak kopyala", onSelect: () => navigator.clipboard.writeText(rowsSel.length > 1 ? `[${rowsSel.map(rowJson).join(",\n")}]` : rowJson(row)) },
      "separator",
      { label: value === null ? `${meta.name} NULL olanları göster` : `${meta.name} = bu değer olanları göster`, onSelect: () => filterBy(meta.name, value) },
    ];
    if (fk) {
      items.push({ label: `${fk.refTable} kaydını aç`, onSelect: () => followFk(meta.name, value), disabled: value === null });
    }
    if (editable) {
      const deleted = row < baseRows && edits.current.deletes.has(row);
      items.push(
        "separator",
        { label: "NULL yap", onSelect: () => setNull([[col, row]]), disabled: !nullable || !columnWritable(col), shortcut: "⌫" },
        { label: rowsSel.length > 1 ? `${rowsSel.length} satırı çoğalt` : "Satırı çoğalt", onSelect: () => duplicateRows(rowsSel) },
        {
          label: deleted ? "Silmeyi geri al" : rowsSel.length > 1 ? `${rowsSel.length} satırı sil` : "Satırı sil",
          onSelect: () => deleteRows(rowsSel),
          danger: !deleted,
        },
      );
    }
    // Glide reports cell bounds in viewport coordinates.
    setMenu({ x: e.bounds.x + e.localEventX, y: e.bounds.y + e.localEventY, items });
  };

  const onHeaderClicked = (col: number) => {
    const name = columns[col]?.name;
    if (!name || !guardPending("Sıralamak")) return;
    setSort((cur) => (cur?.column !== name ? { column: name, descending: false } : cur.descending ? null : { column: name, descending: true }));
  };

  const onHeaderMenu = (col: number, bounds: Rectangle) => {
    const name = columns[col]?.name;
    if (!name) return;
    setMenu({
      x: bounds.x,
      y: bounds.y + bounds.height,
      items: [
        { label: "Artan sırala", onSelect: () => guardPending("Sıralamak") && setSort({ column: name, descending: false }) },
        { label: "Azalan sırala", onSelect: () => guardPending("Sıralamak") && setSort({ column: name, descending: true }) },
        { label: "Sıralamayı kaldır", onSelect: () => setSort(null), disabled: sort?.column !== name },
        "separator",
        {
          label: "Bu sütuna göre filtrele",
          onSelect: () => {
            setShowFilters(true);
            setFilterState((st) => ({ ...st, filters: [...st.filters, { column: name, op: "contains", value: "" }] }));
          },
        },
        { label: "Sütun adını kopyala", onSelect: () => navigator.clipboard.writeText(name) },
      ],
    });
  };

  const onDelete = (sel: GridSelection) => {
    if (!editable) return false;
    if (sel.rows.length > 0) {
      deleteRows(sel.rows.toArray());
      return false;
    }
    if (sel.current) {
      const { x, y, width, height } = sel.current.range;
      const cells: Item[] = [];
      for (let c = x; c < x + width; c++) for (let r = y; r < y + height; r++) cells.push([c, r]);
      setNull(cells);
    }
    return false;
  };

  const onVisibleRegionChanged = (range: Rectangle) => {
    if (!loading && !exhausted && !error && range.y + range.height > rows.current.length - 60) load(false);
  };

  const columnNames = useMemo(() => columns.map((c) => c.name), [columns]);

  return (
    <div className={s.dataPane}>
      {showFilters && (
        <FilterBar
          columns={columnNames.length ? columnNames : design.columns.map((c) => c.name)}
          value={filterState}
          kind={connection.kind}
          onApply={(next) => guardPending("Filtrelemek") && setFilterState(next)}
        />
      )}
      {!editable && !details.isView && !connection.readOnly && (
        <div className={s.banner}>Bu tablonun primary key'i yok, satırlar güvenle tanımlanamadığı için düzenleme kapalı.</div>
      )}
      <div className={s.gridHost}>
        {error ? (
          <div className={s.error} role="alert">
            <b>Veri okunamadı</b>
            <span className="selectable">{error.message}</span>
          </div>
        ) : columns.length > 0 ? (
          <DataEditor
            ref={grid}
            columns={gridColumns}
            rows={totalRows}
            getCellContent={getCellContent}
            onCellEdited={onCellEdited}
            onCellContextMenu={onCellContextMenu}
            onHeaderClicked={onHeaderClicked}
            onHeaderContextMenu={(col, e) => {
              e.preventDefault();
              onHeaderMenu(col, e.bounds);
            }}
            onDelete={onDelete}
            onVisibleRegionChanged={onVisibleRegionChanged}
            onColumnResize={(col, w) => setWidths((cur) => ({ ...cur, [col.id as string]: w }))}
            gridSelection={selection}
            onGridSelectionChange={setSelection}
            getCellsForSelection
            rowMarkers="clickable-number"
            rowSelectionMode="multi"
            smoothScrollX
            smoothScrollY
            rowHeight={26}
            headerHeight={30}
            theme={theme}
            width="100%"
            height="100%"
            overscrollX={40}
            keybindings={{ search: false, copy: true, paste: editable, selectAll: true, delete: editable }}
            onPaste={editable}
            trailingRowOptions={editable ? { hint: "Satır ekle", sticky: false, tint: true } : undefined}
            onRowAppended={editable ? () => addRow() : undefined}
          />
        ) : (
          loading && <div className={s.center}>Yükleniyor…</div>
        )}
      </div>
      <ContextMenu menu={menu} onClose={() => setMenu(null)} />
    </div>
  );
});
