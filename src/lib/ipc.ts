import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  AiFilterResult,
  CsvPreview,
  ExportFormat,
  ImportPlan,
  AiProvider,
  AiSettings,
  AiStatus,
  LocalDatabase,
  PlanNode,
  SshHost,
  SummaryRequest,
  ProviderPreset,
  BrowseRequest,
  DriverInfo,
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
  listDrivers: () => invoke<DriverInfo[]>("list_drivers"),
  listConnections: () => invoke<ConnectionConfig[]>("list_connections"),
  parseConnectionUrl: (url: string) => invoke<ParsedUrl>("parse_connection_url", { url }),
  saveConnection: (config: ConnectionConfig, password: string | null, tunnelSecret: string | null = null) =>
    invoke<ConnectionConfig>("save_connection", { config, password, tunnelSecret }),
  deleteConnection: (id: string) => invoke<void>("delete_connection", { id }),
  testConnection: (config: ConnectionConfig, password: string | null, tunnelSecret: string | null = null) =>
    invoke<TestReport>("test_connection", { config, password, tunnelSecret }),
  forgetHostKey: (host: string, port: number) => invoke<void>("forget_host_key", { host, port }),
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

  aiStatus: () => invoke<AiStatus>("ai_status"),
  aiPresets: () => invoke<ProviderPreset[]>("ai_presets"),
  aiSettings: () => invoke<AiSettings>("ai_settings"),
  saveAiProvider: (provider: AiProvider, key: string | null) => invoke<AiProvider>("save_ai_provider", { provider, key }),
  deleteAiProvider: (id: string) => invoke<void>("delete_ai_provider", { id }),
  setActiveAi: (id: string) => invoke<void>("set_active_ai", { id }),
  aiModels: (provider: AiProvider, key: string | null) => invoke<string[]>("ai_models", { provider, key }),
  discoverLocal: () => invoke<LocalDatabase[]>("discover_local"),
  sshConfigHosts: () => invoke<SshHost[]>("ssh_config_hosts"),
  logUiError: (message: string) => invoke<void>("log_ui_error", { message }),
  diagnostics: () => invoke<string>("diagnostics"),
  summarize: (id: string, request: SummaryRequest) => invoke<Page>("summarize", { id, request }),
  planReplace: (id: string, request: BrowseRequest, column: string, find: string, replacement: string) =>
    invoke<{ statement: string; rows: number }>("plan_replace", { id, request, column, find, replacement }),
  explain: (id: string, sql: string) => invoke<PlanNode>("explain", { id, sql }),
  checkSql: (id: string, sql: string) => invoke<{ writes: boolean; destructive: number }>("check_sql", { id, sql }),
  exportQuery: (id: string, sql: string, format: ExportFormat, path: string) => invoke<number>("export_query", { id, sql, format, path }),
  exportTable: (id: string, request: BrowseRequest, format: ExportFormat, path: string) => invoke<number>("export_table", { id, request, format, path }),
  csvPreview: (path: string, encoding: string | null = null, sheet: string | null = null) => invoke<CsvPreview>("csv_preview", { path, encoding, sheet }),
  importCsv: (id: string, path: string, plan: ImportPlan) => invoke<number>("import_csv", { id, path, plan }),
  aiFilters: (id: string, schema: string | null, table: string, prompt: string, today: string) =>
    invoke<AiFilterResult>("ai_filters", { id, schema, table, prompt, today }),

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
