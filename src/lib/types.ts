// Mirrors of the Rust types in crates/kiyi-core (serde camelCase).

export type DbKind = "postgres" | "mysql" | "sqlite" | "sqlserver";
export type EnvTag = "local" | "staging" | "production";
export type SslMode = "disable" | "prefer" | "require" | "verify-ca" | "verify-full";

export interface ConnectionConfig {
  id: string;
  name: string;
  kind: DbKind;
  host: string;
  port: number;
  user: string;
  database: string | null;
  sslMode: SslMode;
  env: EnvTag;
  readOnly: boolean;
  driver?: string | null;
  tunnel?: TunnelConfig | null;
  /** CA certificate (PEM) to trust for SSL. */
  sslRootCert?: string | null;
  auth?: DbAuth;
}

export type DbAuth = { method: "password" } | { method: "awsIam"; region: string | null; profile: string | null };

export type SshAuth = { method: "agent" } | { method: "key"; path: string } | { method: "password" };

export interface JumpHost {
  host: string;
  port: number;
  user: string;
}

export type TunnelConfig =
  | { type: "ssh"; host: string; port: number; user: string; auth: SshAuth; jump?: JumpHost | null }
  | { type: "ssm"; target: string; region: string | null; profile: string | null }
  | { type: "kubernetes"; target: string; namespace: string | null; context: string | null }
  | { type: "cloudSql"; instance: string };

export interface ParsedUrl {
  config: ConnectionConfig;
  password: string | null;
}

export interface TestStep {
  label: string;
  ok: boolean;
  detail: string | null;
}

export interface TestReport {
  ok: boolean;
  steps: TestStep[];
  serverVersion: string | null;
}

export interface ErrorInfo {
  message: string;
  code: string | null;
  position: number | null;
  statementIndex?: number | null;
}

export type ValueKind = "text" | "number" | "bool" | "json" | "temporal" | "uuid" | "binary" | "array" | "other";

export interface ColumnMeta {
  name: string;
  typeName: string;
  kind: ValueKind;
}

export type Cell = string | null;

export type QueryEvent =
  | { type: "columns"; columns: ColumnMeta[] }
  | { type: "rows"; rows: Cell[][] }
  | { type: "statementDone"; rowsAffected: number }
  | { type: "done"; elapsedMs: number; cancelled: boolean; truncated: boolean }
  | { type: "error"; error: ErrorInfo };

export interface ColumnInfo {
  name: string;
  dataType: string;
  nullable: boolean;
}

export interface TableInfo {
  name: string;
  kind: "table" | "view";
  columns: ColumnInfo[];
  rowEstimate: number | null;
}

export interface SchemaInfo {
  name: string;
  tables: TableInfo[];
}

export interface SchemaSnapshot {
  defaultSchema: string | null;
  schemas: SchemaInfo[];
}

export interface UpdateInfo {
  version: string;
  currentVersion: string;
  notes: string | null;
  date: string | null;
  critical: boolean;
}

// ---- structure

export type FkAction = "noAction" | "restrict" | "cascade" | "setNull" | "setDefault";

export interface ColumnDesign {
  original: string | null;
  name: string;
  dataType: string;
  nullable: boolean;
  default: string | null;
  primaryKey: boolean;
  autoIncrement: boolean;
  comment: string | null;
  generated: boolean;
  extra: string | null;
  enumValues: string[];
}

export interface IndexDesign {
  original: string | null;
  name: string;
  columns: string[];
  unique: boolean;
  isConstraint: boolean;
}

export interface ForeignKeyDesign {
  original: string | null;
  name: string;
  columns: string[];
  refSchema: string | null;
  refTable: string;
  refColumns: string[];
  onDelete: FkAction;
  onUpdate: FkAction;
}

export interface TableDesign {
  name: string;
  columns: ColumnDesign[];
  indexes: IndexDesign[];
  foreignKeys: ForeignKeyDesign[];
  primaryKeyName: string | null;
}

export interface TableDetails {
  schema: string | null;
  design: TableDesign;
  isView: boolean;
  rowEstimate: number | null;
}

export type TableAction = { type: "drop" } | { type: "truncate" } | { type: "rename"; to: string };

// ---- data

export type FilterOp =
  | "eq"
  | "ne"
  | "lt"
  | "gt"
  | "le"
  | "ge"
  | "contains"
  | "notContains"
  | "startsWith"
  | "endsWith"
  | "isNull"
  | "notNull"
  | "in";

export interface Filter {
  column: string;
  op: FilterOp;
  value: string;
}

export interface Sort {
  column: string;
  descending: boolean;
}

export interface BrowseRequest {
  schema: string | null;
  table: string;
  filters: Filter[];
  rawWhere: string | null;
  search?: string | null;
  searchColumns?: string[];
  sort: Sort[];
  tiebreak: string[];
  limit: number;
  offset: number;
}

export interface Page {
  columns: ColumnMeta[];
  rows: Cell[][];
  sql: string;
}

export interface ColumnValue {
  column: string;
  value: Cell;
}

export type RowChange =
  | { type: "update"; key: ColumnValue[]; values: ColumnValue[] }
  | { type: "insert"; values: ColumnValue[] }
  | { type: "delete"; key: ColumnValue[] };

export interface ChangeSet {
  schema: string | null;
  table: string;
  binaryColumns: string[];
  /** True/false columns; SQLite stores them as 1/0. */
  boolColumns?: string[];
  changes: RowChange[];
}

export type ScriptKind = "data" | "schema" | "bulk";

export type ExportFormat = "csv" | "json" | "xlsx";

export interface CsvPreview {
  headers: string[];
  rows: string[][];
  total: number;
  /** Text encoding the file was read with, e.g. "UTF-8" or "windows-1254"; the file type for spreadsheets. */
  encoding: string;
  /** A spreadsheet's sheets (empty for CSV) and the one shown. */
  sheets: string[];
  sheet: string | null;
}

export interface ImportPlan {
  schema: string | null;
  table: string;
  mapping: (string | null)[];
  hasHeader: boolean;
  emptyAsNull: boolean;
  encoding?: string | null;
  sheet?: string | null;
}

// ---- catalog

export type TypeCategory =
  | "text" | "number" | "decimal" | "boolean" | "date" | "dateTime" | "time"
  | "identifier" | "json" | "binary" | "list" | "other";

export interface TypeOption {
  label: string;
  category: TypeCategory;
  sql: string;
  hint: string;
}

export interface DriverInfo {
  id: string;
  name: string;
  kind: DbKind | null;
  defaultPort: number;
  urlExample: string;
  types: TypeOption[];
  recognise: [string, TypeCategory][];
}

// ---- AI

export interface AiStatus {
  configured: boolean;
  provider: string | null;
  model: string | null;
}

export type ProviderKind = "anthropic" | "openAi";

export interface AiProvider {
  id: string;
  name: string;
  kind: ProviderKind;
  baseUrl: string;
  model: string;
  preset: string | null;
}

export interface ProviderView extends AiProvider {
  keySource: "keychain" | "environment" | null;
  needsKey: boolean;
}

export interface AiSettings {
  providers: ProviderView[];
  active: string | null;
}

export interface ProviderPreset {
  id: string;
  name: string;
  kind: ProviderKind;
  baseUrl: string;
  defaultModel: string;
  needsKey: boolean;
  local: boolean;
  keyUrl: string | null;
  envVar: string | null;
  description: string;
}

export interface LocalDatabase {
  kind: DbKind;
  driver: string;
  host: string;
  port: number;
  version: string | null;
  container: string | null;
}

/** A host from ~/.ssh/config. */
export interface SshHost {
  alias: string;
  host: string;
  port: number;
  user: string | null;
  identityFile: string | null;
  jump: JumpHost | null;
}

export interface AiFilterResult {
  filters: Filter[];
  sort: Sort[];
  condition: string | null;
  explanation: string;
}

/** One step of a query plan, the same shape for every database. */
export interface PlanNode {
  label: string;
  detail: string | null;
  rows: number | null;
  cost: number | null;
  warning: string | null;
  children: PlanNode[];
}

/** How the AI suggests drawing a result, by column name. */
export interface ChartHint {
  kind: "bar" | "line" | "pie" | "number";
  x: string | null;
  y: string[];
}

export interface AskResult {
  sql: string;
  explanation: string;
  chart: ChartHint | null;
}

export interface SqlSuggestion {
  sql: string;
  explanation: string;
  writes: boolean;
}

export interface QueryExplanation {
  summary: string;
  steps: string[];
  warnings: string[];
}

export type Aggregate = "count" | "countDistinct" | "sum" | "avg" | "min" | "max";
export type DatePart = "day" | "month" | "year";
export interface GroupBy {
  column: string;
  datePart: DatePart | null;
}
export interface Measure {
  aggregate: Aggregate;
  column: string | null;
}
export interface SummaryRequest {
  browse: BrowseRequest;
  groupBy: GroupBy[];
  measures: Measure[];
}

export interface Relation {
  schema: string;
  table: string;
  columns: string[];
  refSchema: string;
  refTable: string;
  refColumns: string[];
}
export interface SchemaGraph {
  primaryKeys: { schema: string; table: string; columns: string[] }[];
  relations: Relation[];
}

export type ObjectKind = "function" | "procedure" | "trigger" | "sequence" | "user";
export interface DbObject {
  kind: ObjectKind;
  schema: string | null;
  name: string;
  detail: string;
  key: string;
}
export interface ObjectList {
  kinds: ObjectKind[];
  objects: DbObject[];
  notes: string[];
}
export interface ObjectSource {
  definition: string;
  drop: string;
}

export interface BackupTools {
  backup: string | null;
  restore: string | null;
  suggestedName: string;
}
export interface BackupReport {
  method: "native" | "kiyi";
  tool: string;
  tables: number;
  rows: number | null;
  bytes: number;
  note: string | null;
}
export interface RestoreReport {
  tool: string;
  statements: number | null;
}

export type DiffStatus = "same" | "different" | "onlyLeft" | "onlyRight";
export interface TableDiff {
  name: string;
  status: DiffStatus;
  columns: { name: string; left: string | null; right: string | null }[];
  leftRows: number | null;
  rightRows: number | null;
}
export interface Comparison {
  leftSchema: string | null;
  rightSchema: string | null;
  tables: TableDiff[];
  migration: string[] | null;
  note: string | null;
}

// ---- moving data between databases

export type IdMode = "keep" | "renumber";
export type WriteMode = "insert" | "skip" | "update";
export type ValueSource =
  | { type: "column"; column: string }
  | { type: "combine"; columns: string[]; separator: string }
  | { type: "fixed"; value: string | null }
  | { type: "reference"; column: string; schema: string | null; table: string }
  | { type: "default" };
export type Otherwise = { type: "keep" } | { type: "null" } | { type: "value"; value: string };
export type MigrationStep =
  | { type: "trim" }
  | { type: "lower" }
  | { type: "upper" }
  | { type: "replace"; find: string; with: string }
  | { type: "split"; separator: string; part: number; rest: boolean }
  | { type: "map"; pairs: { from: string; to: string | null }[]; otherwise: Otherwise }
  | { type: "ifEmpty"; value: string | null };
export interface ColumnMapping {
  target: string;
  source: ValueSource;
  steps: MigrationStep[];
}
export interface TableMapping {
  enabled: boolean;
  sourceSchema: string | null;
  sourceTable: string;
  targetSchema: string | null;
  targetTable: string;
  columns: ColumnMapping[];
  ids: IdMode;
  write: WriteMode;
  matchOn: string[];
}
export interface MigrationPlan {
  source: string;
  target: string;
  sourceName: string;
  targetName: string;
  tables: TableMapping[];
}
export interface MigrationIssue {
  severity: "error" | "warning";
  table: string;
  column: string | null;
  message: string;
}
export interface MigrationTableCheck {
  source: string;
  target: string;
  rows: number;
  columns: string[];
  preview: Cell[][];
  issues: MigrationIssue[];
}
export interface MigrationCheck {
  tables: MigrationTableCheck[];
  issues: MigrationIssue[];
  order: string[];
  ready: boolean;
}
export interface MigrationProgress {
  stage: "preparing" | "reading" | "writing";
  table: string;
  done: number;
  total: number;
}
export interface MigrationReport {
  tables: { target: string; rows: number }[];
  rows: number;
  testRun: boolean;
  seconds: number;
}
