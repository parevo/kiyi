import { MySQL, PostgreSQL, SQLite } from "@codemirror/lang-sql";
import { classHighlighter, highlightCode } from "@lezer/highlight";
import type { ReactNode } from "react";
import type { DbKind } from "./types";

/** Renders SQL as highlighted spans without an editor instance. */
export function HighlightedSql({ sql, kind }: { sql: string; kind: DbKind }) {
  const dialect = kind === "mysql" ? MySQL : kind === "sqlite" ? SQLite : PostgreSQL;
  const tree = dialect.language.parser.parse(sql);
  const out: ReactNode[] = [];
  highlightCode(
    sql,
    tree,
    classHighlighter,
    (text, classes) => out.push(classes ? <span key={out.length} className={classes}>{text}</span> : text),
    () => out.push("\n"),
  );
  return <>{out}</>;
}

export const isDestructive = (sql: string) =>
  /^\s*(DROP|TRUNCATE|DELETE)\b/i.test(sql) || /^\s*ALTER\s+TABLE\b[\s\S]*\bDROP\s+(COLUMN|CONSTRAINT|FOREIGN|PRIMARY|INDEX)\b/i.test(sql);
