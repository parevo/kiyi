import type { Cell } from "./types";

export interface Summary {
  /** Cells selected. */
  count: number;
  /** Cells that aren't NULL. */
  filled: number;
  /** Present when every filled cell is a number. */
  numbers: { sum: number; avg: number; min: number; max: number } | null;
}

const NUMBER = /^[-+]?(\d+\.?\d*|\.\d+)(e[-+]?\d+)?$/i;

/** Excel-style status-bar figures for a selection. */
export function summarize(values: Cell[]): Summary {
  let filled = 0;
  let sum = 0;
  let min = Infinity;
  let max = -Infinity;
  let numeric = true;
  for (const v of values) {
    if (v === null) continue;
    filled++;
    const t = v.trim();
    if (!numeric || !NUMBER.test(t)) {
      numeric = false;
      continue;
    }
    const n = Number(t);
    sum += n;
    if (n < min) min = n;
    if (n > max) max = n;
  }
  return {
    count: values.length,
    filled,
    numbers: numeric && filled > 0 ? { sum, avg: sum / filled, min, max } : null,
  };
}

const fmt = new Intl.NumberFormat("en-US", { maximumFractionDigits: 4 });

/** "Count 12 · Sum 1,234.5 · Average 102.88 · Min 1 · Max 400", for the status bar. */
export function describeSummary(s: Summary): string {
  if (s.count < 2) return "";
  const parts = [`Count ${fmt.format(s.filled)}`];
  if (s.numbers) {
    const n = s.numbers;
    parts.push(`Sum ${fmt.format(n.sum)}`, `Average ${fmt.format(n.avg)}`, `Min ${fmt.format(n.min)}`, `Max ${fmt.format(n.max)}`);
  }
  return parts.join(" · ");
}
