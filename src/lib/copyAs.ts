import type { Cell, ColumnMeta, DbKind } from "./types";

export type CopyFormat = "tsv" | "csv" | "markdown" | "json" | "insert";

export const COPY_FORMATS: { format: CopyFormat; label: string }[] = [
  { format: "tsv", label: "For Excel / Sheets" },
  { format: "csv", label: "CSV" },
  { format: "markdown", label: "Markdown table" },
  { format: "json", label: "JSON" },
  { format: "insert", label: "SQL INSERT statements" },
];

function csvField(v: Cell, sep: string): string {
  if (v === null) return "";
  return v.includes(sep) || v.includes('"') || v.includes("\n") || v.includes("\r") ? `"${v.replace(/"/g, '""')}"` : v;
}

const ident = (kind: DbKind, name: string) => (kind === "mysql" ? `\`${name.replace(/`/g, "``")}\`` : `"${name.replace(/"/g, '""')}"`);

function literal(kind: DbKind, col: ColumnMeta, v: Cell): string {
  if (v === null) return "NULL";
  if (col.kind === "number" && /^[-+]?(\d+\.?\d*|\.\d+)(e[-+]?\d+)?$/i.test(v)) return v;
  if (col.kind === "bool" && (v === "true" || v === "false")) return kind === "mysql" ? (v === "true" ? "1" : "0") : v.toUpperCase();
  let s = v.replace(/'/g, "''");
  if (kind === "mysql") s = s.replace(/\\/g, "\\\\");
  return `'${s}'`;
}

function jsonValue(col: ColumnMeta, v: Cell): unknown {
  if (v === null) return null;
  if (col.kind === "number" && Number.isFinite(Number(v)) && String(Number(v)) === v) return Number(v);
  if (col.kind === "bool") return v === "true";
  if (col.kind === "json") {
    try {
      return JSON.parse(v);
    } catch {
      return v;
    }
  }
  return v;
}

/** Rows (with the given columns) as text in one of the copy formats. `table` names the INSERT target. */
export function formatRows(format: CopyFormat, columns: ColumnMeta[], rows: Cell[][], opts: { kind: DbKind; table?: string } = { kind: "postgres" }): string {
  switch (format) {
    case "tsv":
      // Excel and Sheets paste tab-separated text into cells; tabs and newlines inside a value are flattened.
      return [columns.map((c) => c.name), ...rows.map((r) => r.map((v) => (v ?? "").replace(/[\t\r\n]+/g, " ")))].map((r) => r.join("\t")).join("\n");
    case "csv":
      return [columns.map((c) => csvField(c.name, ",")), ...rows.map((r) => r.map((v) => csvField(v, ",")))].map((r) => r.join(",")).join("\n");
    case "markdown": {
      const esc = (v: string) => v.replace(/\|/g, "\\|").replace(/\r?\n/g, " ");
      const head = `| ${columns.map((c) => esc(c.name)).join(" | ")} |`;
      const rule = `| ${columns.map((c) => (c.kind === "number" ? "---:" : "---")).join(" | ")} |`;
      return [head, rule, ...rows.map((r) => `| ${r.map((v) => (v === null ? "NULL" : esc(v))).join(" | ")} |`)].join("\n");
    }
    case "json": {
      const objects = rows.map((r) => Object.fromEntries(columns.map((c, i) => [c.name, jsonValue(c, r[i])])));
      return JSON.stringify(objects.length === 1 ? objects[0] : objects, null, 2);
    }
    case "insert": {
      const target = opts.table ?? "my_table";
      const cols = columns.map((c) => ident(opts.kind, c.name)).join(", ");
      return rows.map((r) => `INSERT INTO ${target} (${cols}) VALUES (${r.map((v, i) => literal(opts.kind, columns[i], v)).join(", ")});`).join("\n");
    }
  }
}
