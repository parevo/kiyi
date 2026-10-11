import type { DbKind } from "./types";

/**
 * Named parameters in a query: `WHERE customer_id = :customer_id`. Kiyi asks for their values
 * when the query runs and writes them in as literals, so saved queries can be reused.
 *
 * A name starts with a letter or underscore. Not parameters: anything inside strings, quoted
 * names and comments, Postgres casts (`::text`), MySQL assignments (`:=`), and array slices
 * (`[1:2]`).
 */
interface Found {
  name: string;
  from: number;
  to: number;
}

function scan(sql: string, mysql: boolean): Found[] {
  const out: Found[] = [];
  const n = sql.length;
  let i = 0;
  while (i < n) {
    const c = sql[i];
    const next = sql[i + 1];
    if (c === "-" && next === "-") {
      const end = sql.indexOf("\n", i);
      i = end === -1 ? n : end + 1;
    } else if (c === "#" && mysql) {
      const end = sql.indexOf("\n", i);
      i = end === -1 ? n : end + 1;
    } else if (c === "/" && next === "*") {
      const end = sql.indexOf("*/", i + 2);
      i = end === -1 ? n : end + 2;
    } else if (c === "'" || c === '"' || c === "`" || c === "[") {
      const close = c === "[" ? "]" : c;
      const escapes = c === "'" && mysql;
      i++;
      while (i < n) {
        if (sql[i] === "\\" && escapes) i += 2;
        else if (sql[i] === close && sql[i + 1] === close && c !== "[") i += 2;
        else if (sql[i] === close) {
          i++;
          break;
        } else i++;
      }
    } else if (c === "$") {
      const tag = /^\$[A-Za-z_]?[A-Za-z0-9_]*\$/.exec(sql.slice(i));
      if (tag) {
        const end = sql.indexOf(tag[0], i + tag[0].length);
        i = end === -1 ? n : end + tag[0].length;
      } else i++;
    } else if (c === ":") {
      const prev = sql[i - 1];
      const m = /^[A-Za-z_][A-Za-z0-9_]*/.exec(sql.slice(i + 1));
      if (prev !== ":" && next !== ":" && next !== "=" && !(prev && /[A-Za-z0-9_]/.test(prev)) && m) {
        out.push({ name: m[0], from: i, to: i + 1 + m[0].length });
        i += 1 + m[0].length;
      } else i++;
    } else i++;
  }
  return out;
}

/** The parameter names in a query, each once, in the order they first appear. */
export function findParams(sql: string, kind: DbKind): string[] {
  return [...new Set(scan(sql, kind === "mysql").map((p) => p.name))];
}

/** A typed value as SQL: numbers as they are, NULL for no value, everything else quoted for this database. */
export function literal(value: string | null, kind: DbKind): string {
  if (value === null) return "NULL";
  const v = value.trim();
  if (/^-?\d+(\.\d+)?$/.test(v) && !/^-?0\d/.test(v)) return v;
  let quoted = value.replace(/'/g, "''");
  if (kind === "mysql") quoted = quoted.replace(/\\/g, "\\\\");
  return `${kind === "sqlserver" ? "N" : ""}'${quoted}'`;
}

/** The query with every parameter replaced by its value. */
export function fillParams(sql: string, values: Record<string, string | null>, kind: DbKind): string {
  let out = "";
  let at = 0;
  for (const p of scan(sql, kind === "mysql")) {
    out += sql.slice(at, p.from) + literal(values[p.name] ?? null, kind);
    at = p.to;
  }
  return out + sql.slice(at);
}
