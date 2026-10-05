import "@glideapps/glide-data-grid/dist/index.css";
import {
  DataEditor,
  type GridCell,
  GridCellKind,
  type GridColumn,
  GridColumnIcon,
  type Item,
  type Theme,
} from "@glideapps/glide-data-grid";
import { useCallback, useMemo, useState } from "react";
import type { ColumnMeta, ValueKind } from "../lib/types";
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

export function ResultGrid({ set, rowCount }: { set: ResultSet; rowCount: number }) {
  const theme = useGridTheme();
  const [widths, setWidths] = useState<Record<number, number>>({});

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
    <DataEditor
      columns={columns}
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
  );
}
