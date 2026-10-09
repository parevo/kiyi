import type { DbKind } from "./types";

/** A name quoted for the database: "name", `name` (MySQL) or [name] (SQL Server). */
export function quoteIdent(kind: DbKind, name: string): string {
  if (kind === "mysql") return `\`${name.replace(/`/g, "``")}\``;
  if (kind === "sqlserver") return `[${name.replace(/]/g, "]]")}]`;
  return `"${name.replace(/"/g, '""')}"`;
}

/** sql-formatter's name for the database's SQL. */
export function formatterLanguage(kind: DbKind): "mysql" | "sqlite" | "postgresql" | "transactsql" {
  return kind === "mysql" ? "mysql" : kind === "sqlite" ? "sqlite" : kind === "sqlserver" ? "transactsql" : "postgresql";
}
