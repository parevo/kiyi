/** How a table's columns are shown: their order, which are hidden, and how many stay frozen on the left. */
export interface ColumnLayout {
  /** Column names in display order; columns not listed (added later) follow in table order. */
  order: string[];
  hidden: string[];
  /** Leading visible columns that stay put while scrolling sideways. */
  frozen: number;
}

export const defaultLayout = (): ColumnLayout => ({ order: [], hidden: [], frozen: 0 });

/** Indices into `names` in display order, hidden ones left out. */
export function displayColumns(names: string[], layout: ColumnLayout): number[] {
  const known = layout.order.filter((n) => names.includes(n));
  const rest = names.filter((n) => !known.includes(n));
  return [...known, ...rest].filter((n) => !layout.hidden.includes(n)).map((n) => names.indexOf(n));
}

/** Moves the column shown at `from` to `to` (both display positions). */
export function moveColumn(names: string[], layout: ColumnLayout, from: number, to: number): ColumnLayout {
  const shown = displayColumns(names, layout).map((i) => names[i]);
  const [moved] = shown.splice(from, 1);
  if (moved === undefined) return layout;
  shown.splice(to, 0, moved);
  // Keep hidden columns where they were relative to the rest, at the end of the order.
  return { ...layout, order: [...shown, ...layout.hidden.filter((h) => names.includes(h))] };
}

export function setHidden(layout: ColumnLayout, name: string, hidden: boolean): ColumnLayout {
  const rest = layout.hidden.filter((h) => h !== name);
  return { ...layout, hidden: hidden ? [...rest, name] : rest };
}
