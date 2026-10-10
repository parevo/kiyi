import { format } from "sql-formatter";
import { formatterLanguage } from "./dialect";
import type { DbKind } from "./types";

/** SQL laid out readably (keywords upper case, one clause per line); unchanged if it can't be parsed. */
export function prettySql(sql: string, kind: DbKind): string {
  try {
    return format(sql, { language: formatterLanguage(kind), keywordCase: "upper", tabWidth: 2 });
  } catch {
    return sql;
  }
}

export interface StatementRange {
  from: number;
  to: number;
  text: string;
}

const isWordChar = (c: string | undefined) => !!c && /[A-Za-z0-9_$@#]/.test(c);

/** BEGIN that starts a transaction rather than a block of statements. */
const TRANSACTION_BEGIN = /^\s*(;|$|TRAN\b|TRANSACTION\b|WORK\b|DEFERRED\b|IMMEDIATE\b|EXCLUSIVE\b|ISOLATION\b|READ\b|DISTRIBUTED\b)/i;
/** END IF / END LOOP… close blocks that weren't opened by a counted BEGIN or CASE. */
const UNCOUNTED_END = /^\s+(IF|LOOP|WHILE|REPEAT|FOR)\b/i;

/**
 * Splits SQL into statements on top-level semicolons, skipping over strings, quoted
 * identifiers, comments, Postgres dollar-quoted bodies, and BEGIN … END blocks (MySQL and
 * SQL Server routines, SQLite triggers, CASE … END).
 */
export function splitStatements(sql: string, mysql = false): StatementRange[] {
  const out: StatementRange[] = [];
  let start = 0;
  let i = 0;
  const n = sql.length;
  let depth = 0;

  const push = (end: number) => {
    const raw = sql.slice(start, end);
    const lead = raw.length - raw.trimStart().length;
    const text = raw.trim();
    if (text) out.push({ from: start + lead, to: start + lead + text.length, text });
  };

  while (i < n) {
    const c = sql[i];
    const next = sql[i + 1];
    if (c === "-" && next === "-") {
      const end = sql.indexOf("\n", i);
      i = end === -1 ? n : end + 1;
    } else if (c === "#" && mysql) {
      // MySQL line comment; in Postgres `#` is an operator (`#>`, `#>>`).
      const end = sql.indexOf("\n", i);
      i = end === -1 ? n : end + 1;
    } else if (c === "/" && next === "*") {
      const end = sql.indexOf("*/", i + 2);
      i = end === -1 ? n : end + 2;
    } else if (c === "'" || c === '"' || c === "`") {
      // Backslash escapes only in MySQL strings and Postgres E'…' strings; standard SQL strings treat it as a character.
      const escapes = c !== '"' && (mysql || (c === "'" && /(^|[^A-Za-z0-9_])[Ee]$/.test(sql.slice(Math.max(0, i - 2), i))));
      i++;
      while (i < n) {
        if (sql[i] === "\\" && escapes) i += 2;
        else if (sql[i] === c && sql[i + 1] === c) i += 2;
        else if (sql[i] === c) { i++; break; }
        else i++;
      }
    } else if (c === "$") {
      const tag = /^\$[A-Za-z_]?[A-Za-z0-9_]*\$/.exec(sql.slice(i));
      if (tag) {
        const end = sql.indexOf(tag[0], i + tag[0].length);
        i = end === -1 ? n : end + tag[0].length;
      } else i++;
    } else if (c === ";" && depth === 0) {
      push(i);
      start = i + 1;
      i++;
    } else if (/[A-Za-z]/.test(c) && !isWordChar(sql[i - 1])) {
      let end = i;
      while (isWordChar(sql[end])) end++;
      const word = sql.slice(i, end).toUpperCase();
      const rest = sql.slice(end, end + 24);
      if ((word === "BEGIN" && !TRANSACTION_BEGIN.test(rest)) || word === "CASE") depth++;
      else if (word === "END" && depth > 0 && !UNCOUNTED_END.test(rest)) depth--;
      i = end;
    } else i++;
  }
  push(n);
  return out;
}

/** The statement under the cursor; on blank space after a `;`, the one before it. */
export function statementAt(sql: string, cursor: number, mysql = false): StatementRange | null {
  const all = splitStatements(sql, mysql);
  if (all.length === 0) return null;
  let best = all[0];
  for (const s of all) {
    if (s.from <= cursor) best = s;
    if (cursor <= s.to) break;
  }
  // Cursor sits before the next statement's first char but after `best`'s semicolon.
  const after = all.find((s) => s.from > best.to && s.from <= cursor);
  return after ?? best;
}
