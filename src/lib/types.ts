// Mirrors of the Rust types in crates/kiyi-core (serde camelCase).

export type DbKind = "postgres" | "mysql";
export type EnvTag = "local" | "staging" | "production";
export type SslMode = "disable" | "prefer" | "require" | "verify-full";

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
}

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
  | { type: "done"; elapsedMs: number; cancelled: boolean }
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
  changes: RowChange[];
}

export type ScriptKind = "data" | "schema";

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
  source: "keychain" | "environment" | null;
  model: string;
}

export interface AiFilterResult {
  filters: Filter[];
  sort: Sort[];
  condition: string | null;
  explanation: string;
}
