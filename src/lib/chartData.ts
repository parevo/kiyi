import type { Cell, ColumnMeta } from "./types";

export type ChartKind = "bar" | "line" | "pie" | "number";

export interface ChartConfig {
  kind: ChartKind;
  /** Column for categories / the time axis; null = one value per row. */
  x: number | null;
  /** Numeric columns to plot (at most MAX_SERIES); empty = count rows per category. */
  y: number[];
}

export interface Series {
  name: string;
  values: number[];
}

export interface ChartData {
  kind: ChartKind;
  categories: string[];
  series: Series[];
  /** Why the requested form couldn't be used, shown above the fallback. */
  note: string | null;
  /** True when x is ordered (dates, numbers): keep its order instead of sorting by value. */
  ordered: boolean;
}

export const MAX_SERIES = 4;
const MAX_BARS = 30;
const MAX_SLICES = 6;
const MAX_POINTS = 2_000;

const NUMBER = /^[-+]?(\d+\.?\d*|\.\d+)(e[-+]?\d+)?$/i;
export const isNumeric = (c: ColumnMeta) => c.kind === "number";
const isOrdered = (c: ColumnMeta) => c.kind === "temporal" || c.kind === "number";

/** The AI's suggestion (by column names) as a config for these columns, or null if it doesn't fit. */
export function configFromHint(columns: ColumnMeta[], hint: { kind: ChartKind; x: string | null; y: string[] } | null | undefined): ChartConfig | null {
  if (!hint) return null;
  const idx = (name: string | null) => (name ? columns.findIndex((c) => c.name === name) : -1);
  const y = hint.y.map(idx).filter((i) => i >= 0 && isNumeric(columns[i]));
  const x = idx(hint.x);
  if (hint.kind === "number") return y.length ? { kind: "number", x: null, y: [y[0]] } : null;
  return x >= 0 ? { kind: hint.kind, x, y: y.slice(0, MAX_SERIES) } : null;
}

/** A sensible first chart for a result: time → line, categories → bars, one number → a figure. */
export function suggestChart(columns: ColumnMeta[], rows: Cell[][]): ChartConfig | null {
  const numeric = columns.map((c, i) => (isNumeric(c) ? i : -1)).filter((i) => i >= 0);
  const temporal = columns.findIndex((c) => c.kind === "temporal");
  // Any non-numeric column can name categories (text, enums, booleans, IDs…).
  const label = columns.findIndex((c) => !isNumeric(c) && c.kind !== "temporal" && c.kind !== "json" && c.kind !== "binary");
  if (rows.length === 1 && numeric.length >= 1 && columns.length <= 2) return { kind: "number", x: null, y: [numeric[0]] };
  if (temporal >= 0) return { kind: "line", x: temporal, y: numeric.filter((i) => i !== temporal).slice(0, MAX_SERIES) };
  if (label >= 0) return { kind: "bar", x: label, y: numeric.slice(0, MAX_SERIES) };
  if (numeric.length >= 2) return { kind: "bar", x: numeric[0], y: numeric.slice(1, 1 + MAX_SERIES) };
  return null;
}

/**
 * Turns rows into chart series. Repeated categories are added up (or counted when no measure is
 * picked); unordered categories are sorted biggest first and the tail folds into "Other".
 */
export function buildChart(columns: ColumnMeta[], rows: Cell[][], config: ChartConfig): ChartData {
  const y = config.y.filter((i) => columns[i] && isNumeric(columns[i])).slice(0, MAX_SERIES);
  const num = (v: Cell) => (v !== null && NUMBER.test(v.trim()) ? Number(v) : null);

  if (config.kind === "number" || config.x === null) {
    const col = y[0] ?? null;
    const total = col === null ? rows.length : rows.reduce((s, r) => s + (num(r[col]) ?? 0), 0);
    return { kind: "number", categories: [col === null ? "Rows" : columns[col].name], series: [{ name: col === null ? "Rows" : columns[col].name, values: [total] }], note: null, ordered: false };
  }

  const x = config.x;
  const ordered = isOrdered(columns[x]);
  const names = y.length ? y.map((i) => columns[i].name) : ["Rows"];
  const totals = new Map<string, number[]>();
  for (const r of rows.slice(0, 200_000)) {
    const key = r[x] ?? "(empty)";
    const acc = totals.get(key) ?? names.map(() => 0);
    if (y.length) y.forEach((col, s) => (acc[s] += num(r[col]) ?? 0));
    else acc[0] += 1;
    totals.set(key, acc);
  }

  let entries = [...totals];
  if (ordered) {
    const numericX = columns[x].kind === "number";
    entries.sort(([a], [b]) => (numericX ? Number(a) - Number(b) : a < b ? -1 : a > b ? 1 : 0));
  } else entries.sort((a, b) => b[1][0] - a[1][0]);

  let kind = config.kind;
  let note: string | null = null;
  if (kind === "pie" && (names.length > 1 || entries.some(([, v]) => v[0] < 0))) {
    kind = "bar";
    note = names.length > 1 ? "A pie shows one measure; showing bars instead." : "A pie can't show negative values; showing bars instead.";
  }
  const limit = kind === "pie" ? MAX_SLICES : kind === "bar" ? MAX_BARS : MAX_POINTS;
  if (entries.length > limit) {
    if (ordered) {
      note = `Showing the first ${limit.toLocaleString("en-US")} of ${entries.length.toLocaleString("en-US")} points.`;
      entries = entries.slice(0, limit);
    } else {
      const rest = entries.slice(limit - 1);
      const other = names.map((_, s) => rest.reduce((sum, [, v]) => sum + v[s], 0));
      note = `The ${rest.length.toLocaleString("en-US")} smallest are grouped as “Other”.`;
      entries = [...entries.slice(0, limit - 1), ["Other", other]];
    }
  }
  return {
    kind,
    categories: entries.map(([k]) => k),
    series: names.map((name, s) => ({ name, values: entries.map(([, v]) => v[s]) })),
    note,
    ordered,
  };
}

/**
 * Series too different in size to share one axis (an order count beside revenue): the small one
 * would be a flat line. Those are drawn as separate charts instead of a second axis.
 */
export function needsSeparateCharts(series: Series[]): boolean {
  if (series.length < 2) return false;
  const peaks = series.map((s) => Math.max(...s.values.map(Math.abs))).filter((p) => p > 0);
  return peaks.length > 1 && Math.max(...peaks) / Math.min(...peaks) > 20;
}

/** Round axis ticks: 0, 250, 500… covering [min, max]. */
export function niceTicks(min: number, max: number, count = 5): number[] {
  if (min === max) max = min + 1;
  const span = max - min;
  const raw = span / count;
  const mag = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 2.5, 5, 10].map((m) => m * mag).find((s) => span / s <= count) ?? 10 * mag;
  const start = Math.floor(min / step) * step;
  const out: number[] = [];
  for (let v = start; v <= max + step * 0.001; v += step) out.push(Number(v.toFixed(10)));
  if (out[out.length - 1] < max) out.push(Number((out[out.length - 1] + step).toFixed(10)));
  return out;
}
