/** A table in the diagram: its name and the columns to list (keys first). */
export interface DiagramTable {
  id: string;
  name: string;
  columns: { name: string; type: string; pk: boolean; fk: boolean }[];
  /** Columns not listed, to say "+N more". */
  hidden: number;
}

export interface DiagramEdge {
  from: string;
  fromColumn: string;
  to: string;
  toColumn: string | null;
}

export interface Placed extends DiagramTable {
  x: number;
  y: number;
  w: number;
  h: number;
}

export const NODE_W = 230;
export const HEADER_H = 30;
export const ROW_H = 21;
const GAP_X = 130;
const GAP_Y = 36;

export const nodeHeight = (t: DiagramTable) => HEADER_H + (t.columns.length + (t.hidden ? 1 : 0)) * ROW_H + 8;

/**
 * Layers tables so that referenced tables sit left of the tables pointing at them, orders each
 * layer next to its neighbors, and puts unconnected tables in a grid underneath.
 */
export function layout(tables: DiagramTable[], edges: DiagramEdge[]): { nodes: Placed[]; width: number; height: number } {
  const ids = new Set(tables.map((t) => t.id));
  const parents = new Map<string, Set<string>>();
  const linked = new Set<string>();
  for (const e of edges) {
    if (!ids.has(e.from) || !ids.has(e.to) || e.from === e.to) continue;
    (parents.get(e.from) ?? parents.set(e.from, new Set()).get(e.from)!).add(e.to);
    linked.add(e.from);
    linked.add(e.to);
  }

  // Layer = longest chain of references below the table (cycles are cut by the visiting guard).
  const layerOf = new Map<string, number>();
  const visiting = new Set<string>();
  const depth = (id: string): number => {
    if (layerOf.has(id)) return layerOf.get(id)!;
    if (visiting.has(id)) return 0;
    visiting.add(id);
    let d = 0;
    for (const p of parents.get(id) ?? []) d = Math.max(d, depth(p) + 1);
    visiting.delete(id);
    layerOf.set(id, d);
    return d;
  };
  const connected = tables.filter((t) => linked.has(t.id));
  connected.forEach((t) => depth(t.id));

  const layers: DiagramTable[][] = [];
  for (const t of connected) (layers[layerOf.get(t.id)!] ??= []).push(t);

  // Order each layer by the average position of the tables it references in the layer before.
  const order = new Map<string, number>();
  layers.forEach((layer, li) => {
    if (li > 0) {
      const score = (t: DiagramTable) => {
        const ps = [...(parents.get(t.id) ?? [])].map((p) => order.get(p)).filter((x): x is number => x !== undefined);
        return ps.length ? ps.reduce((a, b) => a + b, 0) / ps.length : Infinity;
      };
      layer.sort((a, b) => score(a) - score(b) || a.name.localeCompare(b.name));
    } else layer.sort((a, b) => a.name.localeCompare(b.name));
    layer.forEach((t, i) => order.set(t.id, i));
  });

  const nodes: Placed[] = [];
  let height = 0;
  layers.forEach((layer, li) => {
    let y = 0;
    for (const t of layer) {
      const h = nodeHeight(t);
      nodes.push({ ...t, x: li * (NODE_W + GAP_X), y, w: NODE_W, h });
      y += h + GAP_Y;
    }
    height = Math.max(height, y);
  });

  // Unconnected tables: a grid below (or alone, when nothing is connected).
  const loose = tables.filter((t) => !linked.has(t.id)).sort((a, b) => a.name.localeCompare(b.name));
  const cols = Math.max(1, Math.min(6, Math.max(layers.length, Math.ceil(Math.sqrt(loose.length)))));
  let rowY = height ? height + GAP_Y * 2 : 0;
  for (let i = 0; i < loose.length; i += cols) {
    const row = loose.slice(i, i + cols);
    const h = Math.max(...row.map(nodeHeight));
    row.forEach((t, j) => nodes.push({ ...t, x: j * (NODE_W + GAP_X / 2), y: rowY, w: NODE_W, h: nodeHeight(t) }));
    rowY += h + GAP_Y;
  }
  const width = Math.max(0, ...nodes.map((n) => n.x + n.w));
  return { nodes, width, height: Math.max(height, rowY) };
}
