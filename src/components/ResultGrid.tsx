import "@glideapps/glide-data-grid/dist/index.css";
import {
  CompactSelection,
  DataEditor,
  type GridCell,
  GridCellKind,
  type GridColumn,
  GridColumnIcon,
  type GridSelection,
  type Item,
  type Rectangle,
  type Theme,
} from "@glideapps/glide-data-grid";
import { useCallback, useEffect, useMemo, useState } from "react";
import { describeSummary, summarize } from "../lib/aggregate";
import { COPY_FORMATS, type CopyFormat, formatRows } from "../lib/copyAs";
import type { Cell, ColumnMeta, DbKind, ValueKind } from "../lib/types";
import { toast } from "../state/toasts";
import { ContextMenu, type MenuState } from "./ContextMenu";
import type { ResultSet } from "../state/tabs";
import { useGridTheme } from "./gridTheme";

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

const CHAR_W = 7.3; // JetBrains Mono at 12px
const MAX_PREVIEW = 400;

/** Width from the header and the first rows, so short columns stay narrow. */
function initialWidth(meta: ColumnMeta, index: number, rows: ResultSet["rows"]): number {
  let longest = meta.name.length + 4;
  for (let r = 0; r < Math.min(rows.length, 60); r++) {
    const v = rows[r][index];
    longest = Math.max(longest, v === null ? 4 : Math.min(v.length, 60));
  }
  return Math.round(Math.min(380, Math.max(72, longest * CHAR_W + 28)));
}

function preview(value: string): string {
  const flat = value.length > MAX_PREVIEW ? value.slice(0, MAX_PREVIEW) + "…" : value;
  return flat.replace(/\r?\n/g, " ⏎ ");
}

export function ResultGrid({
  set,
  rowCount,
  kind,
  onSummary,
}: {
  set: ResultSet;
  rowCount: number;
  kind: DbKind;
  /** Status-bar figures for the selected cells, or "". */
  onSummary?(summary: string): void;
}) {
  const theme = useGridTheme();
  const [widths, setWidths] = useState<Record<number, number>>({});
  const [selection, setSelection] = useState<GridSelection>({ columns: CompactSelection.empty(), rows: CompactSelection.empty() });
  const [menu, setMenu] = useState<MenuState | null>(null);

  useEffect(() => {
    const values: Cell[] = [];
    const ranges = selection.current ? [selection.current.range, ...selection.current.rangeStack] : [];
    for (const { x, y, width, height } of ranges)
      for (let r = y; r < y + height && values.length < 100_000; r++) for (let c = x; c < x + width; c++) values.push(set.rows[r]?.[c] ?? null);
    onSummary?.(describeSummary(summarize(values)));
  }, [selection, set, onSummary]);

  const copyAs = (format: CopyFormat, rows: number[]) => {
    navigator.clipboard.writeText(formatRows(format, set.columns, rows.map((r) => set.rows[r] ?? []), { kind })).then(
      () => toast.success(rows.length === 1 ? "Copied 1 row" : `Copied ${rows.length} rows`),
      () => toast.error("Couldn't copy to the clipboard."),
    );
  };

  const onCellContextMenu = ([col, row]: Item, e: { bounds: Rectangle; localEventX: number; localEventY: number; preventDefault(): void }) => {
    e.preventDefault();
    if (row < 0) return;
    const range = selection.current?.range;
    const rows = selection.rows.hasIndex(row)
      ? selection.rows.toArray()
      : range && row >= range.y && row < range.y + range.height
        ? Array.from({ length: range.height }, (_, i) => range.y + i)
        : [row];
    const x = e.bounds.x + e.localEventX;
    const y = e.bounds.y + e.localEventY;
    setMenu({
      x,
      y,
      items: [
        { label: "Copy value", onSelect: () => navigator.clipboard.writeText(set.rows[row]?.[col] ?? "") },
        ...COPY_FORMATS.map((f) => ({ label: `Copy ${rows.length > 1 ? `${rows.length} rows` : "row"} ${f.format === "tsv" ? "for Excel / Sheets" : `as ${f.label}`}`, onSelect: () => copyAs(f.format, rows) })),
      ],
    });
  };

  const nullTheme = useMemo<Partial<Theme>>(
    () => ({ textDark: getComputedStyle(document.documentElement).getPropertyValue("--null").trim(), baseFontStyle: "italic 12px" }),
    [theme],
  );

  // Widths are decided once rows are present, then only change when the user resizes.
  const sampled = rowCount > 0;
  const columns = useMemo<GridColumn[]>(
    () =>
      set.columns.map((c, i) => ({
        id: String(i),
        title: c.name,
        icon: ICONS[c.kind],
        width: widths[i] ?? initialWidth(c, i, set.rows),
      })),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [set.columns, widths, sampled],
  );

  const getCellContent = useCallback(
    ([col, row]: Item): GridCell => {
      const value = set.rows[row]?.[col] ?? null;
      const kind = set.columns[col]?.kind;
      if (value === null) {
        return { kind: GridCellKind.Text, data: "", displayData: "NULL", allowOverlay: false, readonly: true, copyData: "NULL", themeOverride: nullTheme };
      }
      if (kind === "bool") {
        return { kind: GridCellKind.Boolean, data: value === "true", allowOverlay: false, readonly: true, copyData: value };
      }
      return {
        kind: GridCellKind.Text,
        data: value,
        displayData: preview(value),
        allowOverlay: true,
        readonly: true,
        copyData: value,
        contentAlign: kind === "number" ? "right" : undefined,
      };
    },
    [set, nullTheme],
  );

  return (
    <>
    <DataEditor
      columns={columns}
      gridSelection={selection}
      onGridSelectionChange={setSelection}
      onCellContextMenu={onCellContextMenu}
      rows={rowCount}
      getCellContent={getCellContent}
      getCellsForSelection
      rowMarkers="number"
      smoothScrollX
      smoothScrollY
      rowHeight={26}
      headerHeight={30}
      theme={theme}
      width="100%"
      height="100%"
      overscrollX={40}
      onColumnResize={(_, width, index) => setWidths((w) => ({ ...w, [index]: width }))}
      keybindings={{ search: true, copy: true, selectAll: true }}
    />
    <ContextMenu menu={menu} onClose={() => setMenu(null)} />
    </>
  );
}
