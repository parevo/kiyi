import { ipc } from "./ipc";
import type { Cell, ColumnMeta, ErrorInfo } from "./types";

/** At most this many rows come back for a chart; more wouldn't fit on one anyway. */
const MAX_ROWS = 5_000;

/**
 * Runs one read-only query to the end and returns its first result. Anything that would change
 * data is refused before it reaches the database.
 */
export async function runToEnd(connectionId: string, sql: string): Promise<{ columns: ColumnMeta[]; rows: Cell[][] }> {
  const check = await ipc.checkSql(connectionId, sql);
  if (check.writes) throw new Error("This query changes data; dashboards only show it. Open it in the SQL editor to run it.");
  const queryId = `tile-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
  let columns: ColumnMeta[] | null = null;
  const rows: Cell[][] = [];
  let error: ErrorInfo | null = null;
  await new Promise<void>((resolve, reject) => {
    ipc
      .runQuery(connectionId, queryId, sql, (e) => {
        if (e.type === "columns" && !columns) columns = e.columns;
        else if (e.type === "rows" && columns && rows.length < MAX_ROWS) rows.push(...e.rows.slice(0, MAX_ROWS - rows.length));
        else if (e.type === "error") error = e.error;
        else if (e.type === "done") resolve();
      })
      .catch(reject);
  });
  if (error) throw new Error((error as ErrorInfo).message);
  return { columns: columns ?? [], rows };
}
