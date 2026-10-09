import "@glideapps/glide-data-grid/dist/index.css";
import {
  CompactSelection,
  DataEditor,
  type DataEditorRef,
  type EditableGridCell,
  type GridCell,
  GridCellKind,
  type GridColumn,
  type GridSelection,
  type Item,
  type Rectangle,
  type SpriteMap,
  type Theme,
} from "@glideapps/glide-data-grid";
import { forwardRef, useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import { describeSummary, summarize } from "../lib/aggregate";
import { type ColumnLayout, displayColumns, moveColumn, setHidden } from "../lib/columnLayout";
import { COPY_FORMATS, type CopyFormat, formatRows } from "../lib/copyAs";
import { quoteIdent } from "../lib/dialect";
import { errorMessage, ipc } from "../lib/ipc";
import type { BrowseRequest, Cell, ColumnMeta, ConnectionConfig, ErrorInfo, Filter, ForeignKeyDesign, RowChange, Sort, TableDetails } from "../lib/types";
import { driverFor, useCatalog } from "../state/catalog";
import { useSettings } from "../state/settings";
import { toast } from "../state/toasts";
import { ContextMenu, type MenuEntry, type MenuState } from "./ContextMenu";
import { useGridTheme } from "./gridTheme";
import { RecordPanel } from "./RecordPanel";
import type { PromptRequest } from "./PromptDialog";
import type { ReviewRequest } from "./ReviewDialog";
import s from "./TableView.module.css";
import { isMod, kbd } from "../lib/platform";

const PAGE_SIZE = 300;
const EXACT_COUNT_LIMIT = 5_000_000;

export interface TableQuery {
  filters: Filter[];
  rawWhere: string | null;
  search: string | null;
  sort: Sort | null;
  /** Set when the current filters came from an AI request. */
  ai: { prompt: string; explanation: string } | null;
}

export const emptyQuery = (filters: Filter[] = []): TableQuery => ({ filters, rawWhere: null, search: null, sort: null, ai: null });

export interface TableDataHandle {
  refresh(): void;
  /** The current view (filters, search, sort) as a request, for export. */
  request(): BrowseRequest;
  deleteSelected(): void;
  duplicateSelected(): void;
  save(): void;
  discard(): void;
}

export interface TableDataStatus {
  loaded: number;
  total: number | null;
  totalIsEstimate: boolean;
  /** Unsaved edits; only used on production connections, where edits are staged. */
  pending: number;
  selectedRows: number;
  /** "Count 3 · Sum 12 …" for the selected cells, or "". */
  selection: string;
  loading: boolean;
  elapsedMs: number | null;
  sql: string | null;
}

interface Props {
  connection: ConnectionConfig;
  details: TableDetails;
  query: TableQuery;
  active: boolean;
  onQuery(q: TableQuery): void;
  onStatus(status: TableDataStatus): void;
  onReview(req: ReviewRequest): void;
  onInsert(prefill?: Record<string, Cell>): void;
  onFollow(fk: ForeignKeyDesign, value: string): void;
  onEditStructure(): void;
  layout: ColumnLayout;
  onLayout(layout: ColumnLayout): void;
  onPrompt(req: PromptRequest): void;
}

/** Lucide's key and link glyphs as grid header sprites. */
const sprite =
  (paths: string): SpriteMap[string] =>
  ({ fgColor }) =>
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="${fgColor}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${paths}</svg>`;
const HEADER_ICONS: SpriteMap = {
  key: sprite('<path d="M2.586 17.414A2 2 0 0 0 2 18.828V21a1 1 0 0 0 1 1h3a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h1a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h.172a2 2 0 0 0 1.414-.586l.814-.814a6.5 6.5 0 1 0-4-4z"/><circle cx="16.5" cy="7.5" r=".5"/>'),
  link: sprite('<path d="M9 17H7A5 5 0 0 1 7 7h2"/><path d="M15 7h2a5 5 0 1 1 0 10h-2"/><line x1="8" x2="16" y1="12" y2="12"/>'),
};

function preview(value: string) {
  const flat = value.length > 400 ? value.slice(0, 400) + "…" : value;
  return flat.replace(/\r?\n/g, " ⏎ ");
}

type Updates = Map<number, Map<number, Cell>>;
const cloneUpdates = (u: Updates): Updates => new Map([...u].map(([r, m]) => [r, new Map(m)]));

export const TableData = forwardRef<TableDataHandle, Props>(function TableData(
  { connection, details, query, active, onQuery, onStatus, onReview, onInsert, onFollow, onEditStructure, layout, onLayout, onPrompt },
  ref,
) {
  const theme = useGridTheme();
  const grid = useRef<DataEditorRef>(null);
  const inspectorOpen = useSettings((st) => st.inspectorOpen);
  const developerMode = useSettings((st) => st.developerMode);
  const driver = driverFor(connection, useCatalog((st) => st.drivers));

  const design = details.design;
  const schema = details.schema;
  const pk = useMemo(() => design.columns.filter((c) => c.primaryKey).map((c) => c.name), [design]);
  const editable = !details.isView && !connection.readOnly && pk.length > 0;
  // On production, edits wait for an explicit Save; elsewhere they save as you go.
  const staged = connection.env === "production";

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

  const updates = useRef<Updates>(new Map());
  const [version, setVersion] = useState(0);
  const bump = () => setVersion((v) => v + 1);
  const [selection, setSelection] = useState<GridSelection>({ columns: CompactSelection.empty(), rows: CompactSelection.empty() });
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [widths, setWidths] = useState<Record<string, number>>({});

  const designByName = useMemo(() => new Map(design.columns.map((c) => [c.name, c])), [design]);
  const fkByColumn = useMemo(() => {
    const m = new Map<string, ForeignKeyDesign>();
    for (const fk of design.foreignKeys) if (fk.columns.length === 1) m.set(fk.columns[0], fk);
    return m;
  }, [design]);
  const colIndex = (name: string) => columns.findIndex((c) => c.name === name);
  // The grid shows columns in the user's order without the hidden ones; `shown[displayCol]` is the real index.
  const names = useMemo(() => columns.map((c) => c.name), [columns]);
  const shown = useMemo(() => displayColumns(names, layout), [names, layout]);

  const request = useCallback(
    (offset: number): BrowseRequest => ({
      schema,
      table: design.name,
      filters: query.filters,
      rawWhere: query.rawWhere,
      search: query.search,
      searchColumns: design.columns.filter((c) => !c.generated).map((c) => c.name),
      sort: query.sort ? [query.sort] : [],
      tiebreak: pk,
      limit: PAGE_SIZE,
      offset,
    }),
    [schema, design, query, pk],
  );

  // ---- loading

  const load = useCallback(
    async (reset: boolean) => {
      const gen = reset ? ++generation.current : generation.current;
      setLoading(true);
      if (reset) setError(null);
      const started = performance.now();
      try {
        const page = await ipc.browseTable(connection.id, request(reset ? 0 : rows.current.length));
        if (gen !== generation.current) return;
        if (reset) {
          rows.current = page.rows;
          setColumns(page.columns);
          setLastSql(page.sql);
        } else rows.current.push(...page.rows);
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
    const unfiltered = !query.filters.length && !query.rawWhere && !query.search;
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
  }, [connection.id, request, query, details.rowEstimate]);

  const reload = useCallback(() => {
    load(true);
    loadCount();
  }, [load, loadCount]);

  useEffect(() => {
    updates.current = new Map();
    setSelection({ columns: CompactSelection.empty(), rows: CompactSelection.empty() });
    bump();
    reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query, design]);

  const pending = [...updates.current.values()].reduce((n, m) => n + m.size, 0);

  // ---- reading cells

  const valueAt = (col: number, row: number): Cell => {
    const u = updates.current.get(row);
    if (u?.has(col)) return u.get(col)!;
    return rows.current[row]?.[col] ?? null;
  };
  const keyOf = (row: number) => pk.map((k) => ({ column: k, value: rows.current[row]?.[colIndex(k)] ?? null }));
  const columnWritable = (col: number) => {
    const d = designByName.get(columns[col]?.name ?? "");
    return editable && !!d && !d.generated;
  };

  // ---- saving

  const binaryColumns = () => columns.filter((c) => c.kind === "binary").map((c) => c.name);
  const boolColumns = () => columns.filter((c) => c.kind === "bool").map((c) => c.name);

  const apply = async (changes: RowChange[]) => {
    const statements = await ipc.planRowChanges(connection.id, { schema, table: design.name, binaryColumns: binaryColumns(), boolColumns: boolColumns(), changes });
    await ipc.executeScript(connection.id, statements, "data");
    return statements;
  };

  const updatesToChanges = (u: Updates): RowChange[] =>
    [...u].map(([row, cols]) => ({
      type: "update",
      key: keyOf(row),
      values: [...cols].map(([c, value]) => ({ column: columns[c].name, value })),
    }));

  /** Writes pending edits. Live mode calls this after every edit; staged mode on Save. */
  const commit = async () => {
    const batch = updates.current;
    if (!batch.size) return;
    const changes = updatesToChanges(batch);
    // What to write back if the user hits Undo.
    const undo: RowChange[] = [...batch].map(([row, cols]) => ({
      type: "update",
      key: pk.map((k) => {
        const c = colIndex(k);
        return { column: k, value: cols.has(c) ? cols.get(c)! : (rows.current[row]?.[c] ?? null) };
      }),
      values: [...cols].map(([c]) => ({ column: columns[c].name, value: rows.current[row]?.[c] ?? null })),
    }));
    try {
      await apply(changes);
      for (const [row, cols] of batch) for (const [c, v] of cols) if (rows.current[row]) rows.current[row][c] = v;
      updates.current = new Map();
      bump();
      const n = changes.length;
      toast.success(n === 1 ? "Saved" : `Saved ${n} rows`, {
        label: "Undo",
        run: async () => {
          try {
            await apply(undo);
            reload();
          } catch (e) {
            toast.error(`Couldn't undo: ${errorMessage(e)}`);
          }
        },
      });
    } catch (e) {
      updates.current = new Map();
      bump();
      toast.error(`Not saved: ${errorMessage(e)}`);
    }
  };

  const commitSoon = useRef<number | null>(null);
  const setValue = (col: number, row: number, value: Cell) => {
    const original = rows.current[row]?.[col] ?? null;
    const u = cloneUpdates(updates.current);
    const cols = u.get(row) ?? new Map<number, Cell>();
    if (value === original) cols.delete(col);
    else cols.set(col, value);
    if (cols.size) u.set(row, cols);
    else u.delete(row);
    updates.current = u;
    bump();
    if (!staged) {
      // Coalesce a paste across many cells into one save.
      if (commitSoon.current) clearTimeout(commitSoon.current);
      commitSoon.current = window.setTimeout(commit, 0);
    }
  };

  const saveStaged = async () => {
    if (!updates.current.size) return;
    const changes = updatesToChanges(updates.current);
    const statements = await ipc.planRowChanges(connection.id, { schema, table: design.name, binaryColumns: binaryColumns(), boolColumns: boolColumns(), changes });
    onReview({
      title: "Save changes",
      subtitle: `${design.name} · production`,
      summary: [{ text: `${changes.length} ${changes.length === 1 ? "row" : "rows"} will be updated` }],
      statements,
      action: "Save",
      confirmWord: design.name,
      run: async () => {
        await ipc.executeScript(connection.id, statements, "data");
        updates.current = new Map();
        bump();
        reload();
      },
    });
  };

  const selectedRows = (): number[] => {
    const out = selection.rows.toArray();
    if (out.length) return out;
    if (selection.current) {
      const { y, height } = selection.current.range;
      return Array.from({ length: height }, (_, i) => y + i);
    }
    return [];
  };

  /** Values of the selected cells (ranges, not whole rows), capped so a huge selection stays quick. */
  function selectedValues(): Cell[] {
    const out: Cell[] = [];
    const ranges = selection.current ? [selection.current.range, ...selection.current.rangeStack] : [];
    for (const { x, y, width, height } of ranges) {
      for (let r = y; r < y + height && out.length < 100_000; r++) for (let c = x; c < x + width; c++) out.push(valueAt(shown[c], r));
    }
    return out;
  }

  /** Selected cells as real column indices and rows. */
  const selectedCells = (): [number, number][] => {
    const out: [number, number][] = [];
    const ranges = selection.current ? [selection.current.range, ...selection.current.rangeStack] : [];
    for (const { x, y, width, height } of ranges) for (let c = x; c < x + width; c++) for (let r = y; r < y + height; r++) out.push([shown[c], r]);
    return out;
  };

  const copyAs = (format: CopyFormat, rowIndices: number[]) => {
    const cols = shown.map((i) => columns[i]);
    const text = formatRows(
      format,
      cols,
      rowIndices.map((r) => shown.map((i) => valueAt(i, r))),
      { kind: connection.kind, table: [schema, design.name].filter((p): p is string => !!p).map((p) => quoteIdent(connection.kind, p)).join(".") },
    );
    navigator.clipboard.writeText(text).then(
      () => toast.success(rowIndices.length === 1 ? "Copied 1 row" : `Copied ${rowIndices.length} rows`),
      (e) => toast.error(errorMessage(e)),
    );
  };

  /** Puts one value into many cells, saving like any other edit. */
  const setMany = (cells: [number, number][]) => {
    const targets = cells.filter(([c]) => columnWritable(c));
    if (!targets.length) return toast.info("None of the selected cells can be edited.");
    const allNullable = targets.every(([c]) => designByName.get(columns[c].name)?.nullable);
    onPrompt({
      title: targets.length === 1 ? "Set value" : `Set ${targets.length} cells`,
      subtitle: [...new Set(targets.map(([c]) => columns[c].name))].join(", "),
      fields: [{ label: "New value", nullable: allNullable }],
      action: "Set",
      run: ([value]) => {
        for (const [c, r] of targets) setValue(c, r, value);
      },
    });
  };

  /** Find and replace in one column, across every row the current filters match. */
  const replaceInColumn = (col: number) => {
    const name = columns[col].name;
    onPrompt({
      title: `Replace in ${name}`,
      subtitle: query.filters.length || query.search || query.rawWhere ? "In every row that matches the current filters" : "In every row of the table",
      fields: [{ label: "Find", mono: true }, { label: "Replace with", mono: true }],
      help: "Matches exact text, case included. You'll see how many rows change before anything is saved.",
      action: "Preview",
      run: async ([find, replacement]) => {
        const plan = await ipc.planReplace(connection.id, request(0), name, find ?? "", replacement ?? "");
        if (plan.rows === 0) throw new Error(`No ${name} values contain “${find}”.`);
        onReview({
          title: `Replace in ${name}`,
          subtitle: design.name,
          summary: [{ text: `${plan.rows.toLocaleString("en-US")} ${plan.rows === 1 ? "row" : "rows"}: “${find}” becomes “${replacement}”` }],
          statements: [plan.statement],
          action: `Update ${plan.rows.toLocaleString("en-US")} ${plan.rows === 1 ? "row" : "rows"}`,
          confirmWord: design.name,
          run: async () => {
            await ipc.executeScript(connection.id, [plan.statement], "bulk");
            toast.success(`Updated ${plan.rows.toLocaleString("en-US")} rows`);
            reload();
          },
        });
      },
    });
  };

  const deleteRows = async (indices: number[]) => {
    if (!editable || !indices.length) return;
    const changes: RowChange[] = indices.map((r) => ({ type: "delete", key: keyOf(r) }));
    try {
      const statements = await ipc.planRowChanges(connection.id, { schema, table: design.name, binaryColumns: binaryColumns(), boolColumns: boolColumns(), changes });
      const n = indices.length;
      onReview({
        title: n === 1 ? "Delete row" : `Delete ${n} rows`,
        subtitle: design.name,
        summary: [{ text: `${n} ${n === 1 ? "row" : "rows"} will be permanently deleted from ${design.name}`, danger: true }],
        statements,
        action: n === 1 ? "Delete row" : `Delete ${n} rows`,
        confirmWord: design.name,
        run: async () => {
          await ipc.executeScript(connection.id, statements, "data");
          toast.success(n === 1 ? "Row deleted" : `${n} rows deleted`);
          reload();
        },
      });
    } catch (e) {
      toast.error(errorMessage(e));
    }
  };

  const duplicate = (row: number) => {
    const prefill: Record<string, Cell> = {};
    columns.forEach((c, i) => {
      const d = designByName.get(c.name);
      if (!d || d.autoIncrement || d.generated || (d.primaryKey && d.default)) return;
      prefill[c.name] = valueAt(i, row);
    });
    onInsert(prefill);
  };

  useImperativeHandle(ref, () => ({
    request: () => request(0),
    refresh: () => {
      if (updates.current.size && !confirm("Discard unsaved changes and reload?")) return;
      updates.current = new Map();
      reload();
    },
    deleteSelected: () => deleteRows(selectedRows()),
    duplicateSelected: () => {
      const r = selectedRows()[0];
      if (r !== undefined) duplicate(r);
    },
    save: saveStaged,
    discard: () => {
      updates.current = new Map();
      bump();
    },
  }));

  // ---- status for the parent toolbar; only report real changes to avoid a render loop

  const status: TableDataStatus = {
    loaded: rowCount,
    total: total?.n ?? null,
    totalIsEstimate: total?.estimate ?? false,
    pending,
    selectedRows: selection.rows.length,
    selection: describeSummary(summarize(selectedValues())),
    loading,
    elapsedMs: elapsed,
    sql: lastSql,
  };
  const lastStatus = useRef("");
  useEffect(() => {
    const key = JSON.stringify(status);
    if (key === lastStatus.current) return;
    lastStatus.current = key;
    onStatus(status);
  });

  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent) => {
      if (!isMod(e)) return;
      const k = e.key.toLowerCase();
      if (k === "s" && staged) {
        e.preventDefault();
        saveStaged();
      } else if (k === "r") {
        e.preventDefault();
        reload();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  // ---- grid

  const colors = useMemo(() => {
    const v = (n: string) => getComputedStyle(document.documentElement).getPropertyValue(n).trim();
    return { edit: v("--edit-bg"), nul: v("--null"), faint: v("--text-faint") };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [theme]);

  const gridTheme = useMemo<Partial<Theme>>(() => ({ ...theme, bgIconHeader: "transparent", fgIconHeader: colors.faint }), [theme, colors]);

  const gridColumns = useMemo<GridColumn[]>(
    () =>
      shown.map((i) => {
        const c = columns[i];
        const d = designByName.get(c.name);
        const arrow = query.sort?.column === c.name ? (query.sort.descending ? "  ↓" : "  ↑") : "";
        let width = widths[c.name];
        if (!width) {
          let longest = c.name.length + 6;
          for (let r = 0; r < Math.min(rows.current.length, 60); r++) {
            const v = rows.current[r][i];
            longest = Math.max(longest, v === null ? 4 : Math.min(v.length, 60));
          }
          width = Math.round(Math.min(380, Math.max(90, longest * 7.3 + 30)));
        }
        return { id: c.name, title: c.name + arrow, icon: d?.primaryKey ? "key" : fkByColumn.has(c.name) ? "link" : undefined, width };
      }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [columns, shown, query.sort, widths, designByName, fkByColumn, rowCount > 0],
  );

  const getCellContent = useCallback(
    ([displayCol, row]: Item): GridCell => {
      const col = shown[displayCol];
      const meta = columns[col];
      const edited = !!updates.current.get(row)?.has(col);
      const value = valueAt(col, row);
      const writable = columnWritable(col);
      const over: Partial<Theme> = edited ? { bgCell: colors.edit } : {};

      if (meta?.kind === "bool") {
        return { kind: GridCellKind.Boolean, data: value === null ? null : value === "true", allowOverlay: false, readonly: !writable, copyData: value ?? "NULL", themeOverride: over };
      }
      if (value === null) {
        return { kind: GridCellKind.Text, data: "", displayData: "NULL", allowOverlay: writable, readonly: !writable, copyData: "NULL", themeOverride: { ...over, textDark: colors.nul, baseFontStyle: "italic 12px" } };
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
    // `version` captures edits; the ref contents are read at draw time.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [columns, shown, version, colors, editable],
  );

  const onCellEdited = ([displayCol, row]: Item, cell: EditableGridCell) => {
    const col = shown[displayCol];
    if (cell.kind === GridCellKind.Boolean) setValue(col, row, cell.data === null || cell.data === undefined ? null : String(cell.data));
    else if (cell.kind === GridCellKind.Text) {
      // Opening a NULL cell and closing it without typing isn't an edit.
      if (cell.data === "" && valueAt(col, row) === null) return;
      setValue(col, row, cell.data);
    }
  };

  const filterBy = (column: string, value: Cell) => {
    const f: Filter = value === null ? { column, op: "isNull", value: "" } : { column, op: "eq", value };
    onQuery({ ...query, ai: null, filters: [...query.filters.filter((x) => x.column !== column), f] });
  };


  const onCellContextMenu = ([displayCol, row]: Item, e: { bounds: Rectangle; localEventX: number; localEventY: number; preventDefault(): void }) => {
    e.preventDefault();
    if (row < 0 || displayCol < 0) return;
    const col = shown[displayCol];
    const meta = columns[col];
    const value = valueAt(col, row);
    const fk = fkByColumn.get(meta.name);
    const nullable = designByName.get(meta.name)?.nullable;
    const rowsSel = selection.rows.hasIndex(row) ? selection.rows.toArray() : [row];
    // Glide reports cell bounds in viewport coordinates.
    const x = e.bounds.x + e.localEventX;
    const y = e.bounds.y + e.localEventY;
    const range = selection.current?.range;
    const inRange = !!range && displayCol >= range.x && displayCol < range.x + range.width && row >= range.y && row < range.y + range.height;
    const cells = inRange ? selectedCells() : ([[col, row]] as [number, number][]);
    const items: MenuEntry[] = [
      { label: "Copy value", onSelect: () => navigator.clipboard.writeText(value ?? ""), shortcut: kbd("C") },
      {
        label: rowsSel.length > 1 ? `Copy ${rowsSel.length} rows as…` : "Copy row as…",
        onSelect: () => setMenu({ x, y, items: COPY_FORMATS.map((f) => ({ label: f.label, onSelect: () => copyAs(f.format, rowsSel) })) }),
      },
      "separator",
      { label: value === null ? `Show rows where ${meta.name} is empty` : `Show rows with this ${meta.name}`, onSelect: () => filterBy(meta.name, value) },
    ];
    if (fk && value !== null) items.push({ label: `Open linked ${fk.refTable} row`, onSelect: () => onFollow(fk, value) });
    if (editable) {
      items.push(
        "separator",
        { label: "Set to NULL", onSelect: () => setValue(col, row, null), disabled: !nullable || !columnWritable(col) || value === null },
        { label: cells.length > 1 ? `Set ${cells.length} cells to…` : "Set value…", onSelect: () => setMany(cells), disabled: !cells.some(([c]) => columnWritable(c)) },
        ...(meta.kind === "text" && columnWritable(col) ? [{ label: `Replace in ${meta.name}…`, onSelect: () => replaceInColumn(col) }] : []),
        { label: "Duplicate row", onSelect: () => duplicate(row) },
        { label: rowsSel.length > 1 ? `Delete ${rowsSel.length} rows…` : "Delete row…", onSelect: () => deleteRows(rowsSel), danger: true },
      );
    }
    setMenu({ x, y, items });
  };

  const onHeaderClicked = (displayCol: number) => {
    const name = columns[shown[displayCol]]?.name;
    if (!name) return;
    const cur = query.sort;
    onQuery({ ...query, sort: cur?.column !== name ? { column: name, descending: false } : cur.descending ? null : { column: name, descending: true } });
  };

  const onDelete = (sel: GridSelection) => {
    if (!editable) return false;
    if (sel.rows.length > 0) {
      deleteRows(sel.rows.toArray());
      return false;
    }
    if (sel.current) {
      const { x, y, width, height } = sel.current.range;
      for (let dc = x; dc < x + width; dc++) {
        const c = shown[dc];
        for (let r = y; r < y + height; r++) if (columnWritable(c) && designByName.get(columns[c].name)?.nullable) setValue(c, r, null);
      }
    }
    return false;
  };

  const onHeaderContextMenu = (displayCol: number, e: { bounds: Rectangle; localEventX: number; localEventY: number; preventDefault(): void }) => {
    e.preventDefault();
    const col = shown[displayCol];
    const meta = columns[col];
    if (!meta) return;
    const frozen = Math.min(layout.frozen, shown.length);
    const items: MenuEntry[] = [
      { label: "Sort ascending", onSelect: () => onQuery({ ...query, sort: { column: meta.name, descending: false } }) },
      { label: "Sort descending", onSelect: () => onQuery({ ...query, sort: { column: meta.name, descending: true } }) },
      "separator",
      { label: `Hide ${meta.name}`, onSelect: () => onLayout(setHidden(layout, meta.name, true)), disabled: shown.length <= 1 },
      displayCol < frozen
        ? { label: "Unfreeze columns", onSelect: () => onLayout({ ...layout, frozen: 0 }) }
        : { label: displayCol === 0 ? "Freeze this column" : "Freeze columns up to here", onSelect: () => onLayout({ ...layout, frozen: displayCol + 1 }) },
    ];
    if (layout.hidden.length) items.push({ label: `Show ${layout.hidden.length} hidden ${layout.hidden.length === 1 ? "column" : "columns"}`, onSelect: () => onLayout({ ...layout, hidden: [] }) });
    if (meta.kind === "text" && columnWritable(col)) items.push("separator", { label: `Replace in ${meta.name}…`, onSelect: () => replaceInColumn(col) });
    setMenu({ x: e.bounds.x + e.localEventX, y: e.bounds.y + e.localEventY, items });
  };

  const onVisibleRegionChanged = (range: Rectangle) => {
    if (!loading && !exhausted && !error && range.y + range.height > rows.current.length - 60) load(false);
  };

  const currentRow = selection.current ? selection.current.cell[1] : selection.rows.length === 1 ? selection.rows.first()! : null;

  return (
    <div className={s.dataPane}>
      {!editable && !details.isView && !connection.readOnly && (
        <div className={s.banner}>This table has no primary key, so rows can't be told apart safely. Editing is turned off.</div>
      )}
      <div className={s.split}>
        <div className={s.gridHost}>
          {error ? (
            <div className={s.error} role="alert">
              <b>Couldn't load rows</b>
              <span className="selectable">{error.message}</span>
            </div>
          ) : columns.length > 0 ? (
            <>
              <DataEditor
                ref={grid}
                columns={gridColumns}
                rows={rowCount}
                getCellContent={getCellContent}
                onCellEdited={onCellEdited}
                onCellContextMenu={onCellContextMenu}
                onHeaderClicked={onHeaderClicked}
                onHeaderContextMenu={onHeaderContextMenu}
                onColumnMoved={(from, to) => onLayout(moveColumn(names, layout, from, to))}
                freezeColumns={Math.min(layout.frozen, shown.length)}
                onDelete={onDelete}
                onVisibleRegionChanged={onVisibleRegionChanged}
                onColumnResize={(c, w) => setWidths((cur) => ({ ...cur, [c.id as string]: w }))}
                gridSelection={selection}
                onGridSelectionChange={setSelection}
                getCellsForSelection
                headerIcons={HEADER_ICONS}
                rowMarkers="clickable-number"
                rowSelectionMode="multi"
                smoothScrollX
                smoothScrollY
                rowHeight={30}
                headerHeight={34}
                theme={gridTheme}
                width="100%"
                height="100%"
                overscrollX={40}
                keybindings={{ search: false, copy: true, paste: editable, selectAll: true, delete: editable }}
                onPaste={editable}
              />
              {!loading && rowCount === 0 && (
                <div className={s.emptyRows}>
                  <b>No rows{query.filters.length || query.search || query.rawWhere ? " match" : " yet"}</b>
                  {query.filters.length || query.search || query.rawWhere ? (
                    <button className={s.linkButton} onClick={() => onQuery({ ...query, filters: [], search: null, rawWhere: null, ai: null })}>
                      Clear filters
                    </button>
                  ) : (
                    editable && (
                      <button className={s.linkButton} onClick={() => onInsert()}>
                        Insert the first row
                      </button>
                    )
                  )}
                </div>
              )}
            </>
          ) : (
            loading && <div className={s.center}>Loading…</div>
          )}
        </div>
        {inspectorOpen && columns.length > 0 && (
          <RecordPanel
            details={details}
            driver={driver}
            columns={columns}
            row={currentRow}
            rowCount={rowCount}
            total={total?.n ?? null}
            value={valueAt}
            edited={(col, row) => !!updates.current.get(row)?.has(col)}
            writable={columnWritable}
            onChange={setValue}
            onFollow={onFollow}
            onMove={(row) => {
              const col = selection.current?.cell[0] ?? 0;
              setSelection({ columns: CompactSelection.empty(), rows: CompactSelection.empty(), current: { cell: [col, row], range: { x: col, y: row, width: 1, height: 1 }, rangeStack: [] } });
              grid.current?.scrollTo(col, row);
            }}
            onEditStructure={onEditStructure}
            developerMode={developerMode}
          />
        )}
      </div>
      <ContextMenu menu={menu} onClose={() => setMenu(null)} />
    </div>
  );
});
