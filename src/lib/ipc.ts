import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  BrowseRequest,
  ChangeSet,
  ConnectionConfig,
  Page,
  ParsedUrl,
  QueryEvent,
  SchemaSnapshot,
  ScriptKind,
  TableAction,
  TableDesign,
  TableDetails,
  TestReport,
  UpdateInfo,
} from "./types";

export const ipc = {
  listConnections: () => invoke<ConnectionConfig[]>("list_connections"),
  parseConnectionUrl: (url: string) => invoke<ParsedUrl>("parse_connection_url", { url }),
  saveConnection: (config: ConnectionConfig, password: string | null) =>
    invoke<ConnectionConfig>("save_connection", { config, password }),
  deleteConnection: (id: string) => invoke<void>("delete_connection", { id }),
  testConnection: (config: ConnectionConfig, password: string | null) =>
    invoke<TestReport>("test_connection", { config, password }),
  connect: (id: string) => invoke<{ serverVersion: string }>("connect", { id }),
  disconnect: (id: string) => invoke<void>("disconnect", { id }),
  loadSchema: (id: string) => invoke<SchemaSnapshot>("load_schema", { id }),

  runQuery(connectionId: string, queryId: string, sql: string, onEvent: (e: QueryEvent) => void) {
    const channel = new Channel<QueryEvent>();
    channel.onmessage = onEvent;
    return invoke<void>("run_query", { connectionId, queryId, sql, onEvent: channel });
  },
  cancelQuery: (queryId: string) => invoke<void>("cancel_query", { queryId }),

  tableDetails: (id: string, schema: string | null, table: string) =>
    invoke<TableDetails>("table_details", { id, schema, table }),
  browseTable: (id: string, request: BrowseRequest) => invoke<Page>("browse_table", { id, request }),
  countRows: (id: string, request: BrowseRequest) => invoke<number>("count_rows", { id, request }),
  planRowChanges: (id: string, changes: ChangeSet) => invoke<string[]>("plan_row_changes", { id, changes }),
  planTable: (id: string, schema: string | null, old: TableDesign | null, next: TableDesign) =>
    invoke<string[]>("plan_table", { id, schema, old, new: next }),
  planTableAction: (id: string, schema: string | null, table: string, isView: boolean, action: TableAction) =>
    invoke<string[]>("plan_table_action", { id, schema, table, isView, action }),
  executeScript: (id: string, statements: string[], kind: ScriptKind) =>
    invoke<number[]>("execute_script", { id, statements, kind }),

  checkUpdate: (channel: "stable" | "beta") => invoke<UpdateInfo | null>("check_update", { channel }),
  downloadUpdate(onProgress: (downloaded: number, total: number | null) => void) {
    const channel = new Channel<{ type: "progress"; downloaded: number; total: number | null }>();
    channel.onmessage = (e) => onProgress(e.downloaded, e.total);
    return invoke<void>("download_update", { onEvent: channel });
  },
  installUpdate: () => invoke<void>("install_update"),
};

/** Commands reject with the Rust `ErrorInfo` (or a plain string from the updater). */
export function errorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err && typeof err === "object" && "message" in err) return String((err as { message: unknown }).message);
  return String(err);
}
