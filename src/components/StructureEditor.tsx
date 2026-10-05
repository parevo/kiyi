import { type ReactNode, useEffect, useMemo, useState } from "react";
import { errorMessage, ipc } from "../lib/ipc";
import { isDestructive } from "../lib/highlight";
import type {
  ColumnDesign,
  ConnectionConfig,
  DriverInfo,
  FkAction,
  ForeignKeyDesign,
  IndexDesign,
  SchemaSnapshot,
  TableDesign,
  TableDetails,
} from "../lib/types";
import { columnTypeLabel, driverFor, friendlyType, useCatalog } from "../state/catalog";
import { useSettings } from "../state/settings";
import { toast } from "../state/toasts";
import { EditIcon, KeyIcon, LinkIcon, PlusIcon, TrashIcon } from "./icons";
import type { ReviewRequest } from "./ReviewDialog";
import { Sheet } from "./Sheet";
import { Button, IconButton, Switch } from "./ui";
import f from "./Form.module.css";
import s from "./StructureEditor.module.css";

const fmt = new Intl.NumberFormat("en-US");
const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
const sqlString = (v: string) => `'${v.replace(/'/g, "''")}'`;

const blankColumn = (name = ""): ColumnDesign => ({
  original: null,
  name,
  dataType: "",
  nullable: true,
  default: null,
  primaryKey: false,
  autoIncrement: false,
  comment: null,
  generated: false,
  extra: null,
  enumValues: [],
});

const ON_DELETE: { value: FkAction; label: string }[] = [
  { value: "noAction", label: "Block the delete" },
  { value: "cascade", label: "Delete this row too" },
  { value: "setNull", label: "Clear this field" },
];
const onDeleteLabel = (a: FkAction) => ON_DELETE.find((o) => o.value === a)?.label ?? a;

// ---- defaults in plain language

type DefaultMode = "none" | "value" | "now" | "auto" | "uuid" | "custom";

function defaultMode(c: ColumnDesign): DefaultMode {
  if (c.autoIncrement) return "auto";
  const d = c.default?.trim();
  if (!d) return "none";
  if (/^nextval\(/i.test(d)) return "auto";
  if (/^(now\(\)|current_timestamp(\(\d*\))?|current_date)$/i.test(d)) return "now";
  if (/gen_random_uuid\(\)|^\(?uuid\(\)\)?$/i.test(d)) return "uuid";
  if (/^'([^']|'')*'(::[\w\s"]+)?$/.test(d) || /^-?\d+(\.\d+)?$/.test(d) || /^(true|false)$/i.test(d)) return "value";
  return "custom";
}

function literalOf(d: string | null): string {
  if (!d) return "";
  const m = /^'((?:[^']|'')*)'(::[\w\s"]+)?$/.exec(d.trim());
  return m ? m[1].replace(/''/g, "'") : d.trim();
}

export function describeDefault(c: ColumnDesign): string {
  switch (defaultMode(c)) {
    case "none":
      return "";
    case "auto":
      return "Auto-increment";
    case "now":
      return /current_date/i.test(c.default ?? "") ? "Today" : "Current time";
    case "uuid":
      return "Random UUID";
    case "value":
      return literalOf(c.default);
    default:
      return c.default ?? "";
  }
}

// ---- plain-language summary of a design change

function summarise(original: TableDesign | null, next: TableDesign, driver: DriverInfo | undefined) {
  const out: { text: string; danger?: boolean }[] = [];
  const ft = (t: string) => friendlyType(t, driver);
  if (!original) {
    out.push({ text: `Create table “${next.name}” with ${next.columns.length} columns` });
    return out;
  }
  if (next.name !== original.name) out.push({ text: `Rename the table to “${next.name}”` });
  const byOriginal = new Map(next.columns.filter((c) => c.original).map((c) => [c.original!, c]));
  for (const o of original.columns) {
    const c = byOriginal.get(o.name);
    if (!c) {
      out.push({ text: `Delete column “${o.name}” and all of its data`, danger: true });
      continue;
    }
    if (c.name !== o.name) out.push({ text: `Rename “${o.name}” to “${c.name}”` });
    if (c.dataType !== o.dataType) out.push({ text: `Change “${c.name}” from ${ft(o.dataType)} to ${ft(c.dataType)}; existing values are converted`, danger: true });
    if (c.nullable !== o.nullable) out.push({ text: c.nullable ? `“${c.name}” becomes optional` : `“${c.name}” becomes required` });
    if (c.default !== o.default || c.autoIncrement !== o.autoIncrement) out.push({ text: `Change the default for “${c.name}”` });
    if (c.primaryKey !== o.primaryKey) out.push({ text: c.primaryKey ? `Make “${c.name}” the primary key` : `“${c.name}” is no longer the primary key` });
    if (c.comment !== o.comment) out.push({ text: `Update the description of “${c.name}”` });
  }
  for (const c of next.columns.filter((c) => !c.original)) out.push({ text: `Add column “${c.name}” (${ft(c.dataType)})` });
  for (const i of next.indexes) {
    const o = original.indexes.find((x) => x.name === i.original);
    if (!o || !same(o, i)) out.push({ text: i.unique ? `Require unique values in ${i.columns.join(", ")}` : `Add an index on ${i.columns.join(", ")} for faster lookups` });
  }
  for (const o of original.indexes) {
    if (!next.indexes.some((i) => i.original === o.name)) out.push({ text: o.unique ? `Stop requiring unique ${o.columns.join(", ")}` : `Remove index “${o.name}”` });
  }
  for (const k of next.foreignKeys) {
    const o = original.foreignKeys.find((x) => x.name === k.original);
    if (!o || !same(o, k)) out.push({ text: `Link ${k.columns.join(", ")} to ${k.refTable}` });
  }
  for (const o of original.foreignKeys) {
    if (!next.foreignKeys.some((k) => k.original === o.name)) out.push({ text: `Remove the link from ${o.columns.join(", ")} to ${o.refTable}` });
  }
  return out;
}

/** Plans `next` against `original` and applies it, asking first when something is lost. */
async function applyDesign({
  connection,
  schema,
  original,
  next,
  driver,
  onReview,
  title,
  done,
}: {
  connection: ConnectionConfig;
  schema: string | null;
  original: TableDesign | null;
  next: TableDesign;
  driver: DriverInfo | undefined;
  onReview(r: ReviewRequest): void;
  title: string;
  done(): void | Promise<void>;
}) {
  const statements = await ipc.planTable(connection.id, schema, original, next);
  if (!statements.length) return done();
  const summary = summarise(original, next, driver);
  const run = async () => {
    await ipc.executeScript(connection.id, statements, "schema");
    await done();
  };
  const risky = summary.some((x) => x.danger) || statements.some(isDestructive);
  if (!risky && connection.env !== "production" && !useSettings.getState().developerMode) {
    await run();
    toast.success(summary.length === 1 ? `Done: ${summary[0].text}` : `Applied ${summary.length} changes`);
    return;
  }
  onReview({
    title,
    subtitle: original?.name ?? next.name,
    summary,
    statements,
    action: risky ? "Apply anyway" : "Apply",
    confirmWord: original?.name,
    nonTransactional: connection.kind === "mysql",
    run,
  });
}

function tablesOf(snapshot: SchemaSnapshot | undefined) {
  const out: { schema: string; name: string; columns: string[] }[] = [];
  for (const sc of snapshot?.schemas ?? []) for (const t of sc.tables) if (t.kind === "table") out.push({ schema: sc.name, name: t.name, columns: t.columns.map((c) => c.name) });
  return out;
}

// =====================================================================================
// Structure of an existing table

type SheetState = { kind: "column"; index: number | null } | { kind: "index" } | { kind: "link" } | { kind: "rename" } | null;

export function StructureView({
  connection,
  details,
  snapshot,
  onReview,
  onChanged,
}: {
  connection: ConnectionConfig;
  details: TableDetails;
  snapshot: SchemaSnapshot | undefined;
  onReview(r: ReviewRequest): void;
  /** Called after a change was applied; `name` is set when the table was renamed. */
  onChanged(name?: string): void | Promise<void>;
}) {
  const driver = driverFor(connection, useCatalog((st) => st.drivers));
  const developerMode = useSettings((st) => st.developerMode);
  const design = details.design;
  const schema = details.schema;
  const readOnly = connection.readOnly || details.isView;
  const [section, setSection] = useState<"columns" | "indexes" | "links">("columns");
  const [sheet, setSheet] = useState<SheetState>(null);
  const tables = useMemo(() => tablesOf(snapshot), [snapshot]);

  const apply = (next: TableDesign, title: string) =>
    applyDesign({ connection, schema, original: design, next, driver, onReview, title, done: () => onChanged(next.name) }).catch((e) => {
      toast.error(errorMessage(e));
      throw e;
    });

  const uniqueOf = (name: string) => design.indexes.some((x) => x.unique && x.columns.length === 1 && x.columns[0] === name);
  const linkOf = (name: string) => design.foreignKeys.find((k) => k.columns.length === 1 && k.columns[0] === name);

  return (
    <div className={s.page}>
      <div className={s.header}>
        <div>
          <h2 className={s.title}>{design.name}</h2>
          <p className={s.meta}>
            {design.columns.length} columns
            {details.rowEstimate !== null && ` · ~${fmt.format(details.rowEstimate)} rows`}
            {details.isView && " · view (read-only)"}
          </p>
        </div>
        {!readOnly && (
          <div className={s.actions}>
            <Button onPress={() => setSheet({ kind: "rename" })}>Rename table</Button>
            <Button variant="primary" onPress={() => setSheet({ kind: "column", index: null })}>
              <PlusIcon size={14} /> Add column
            </Button>
          </div>
        )}
      </div>

      <div className={s.tabs} role="tablist">
        {(
          [
            ["columns", `Columns`, design.columns.length],
            ["indexes", `Indexes`, design.indexes.length],
            ["links", `Relationships`, design.foreignKeys.length],
          ] as const
        ).map(([id, label, n]) => (
          <button key={id} role="tab" aria-selected={section === id} onClick={() => setSection(id)}>
            {label} <span className={s.tabCount}>{n}</span>
          </button>
        ))}
      </div>

      {section === "columns" && (
        <div className={s.table} role="table">
          <div className={s.thead} role="row">
            <span>Name</span>
            <span>Type</span>
            <span>Default</span>
            <span>Rules</span>
            <span />
          </div>
          {design.columns.map((c, i) => {
            const link = linkOf(c.name);
            const editableCol = !readOnly && !c.generated;
            return (
              <div key={c.name} className={s.tr} role="row" onClick={() => editableCol && setSheet({ kind: "column", index: i })} data-clickable={editableCol || undefined}>
                <span className={s.name}>
                  {c.primaryKey && <KeyIcon size={13} />}
                  {c.name}
                </span>
                <span className={s.type}>
                  {columnTypeLabel(c, driver)}
                  {(developerMode || columnTypeLabel(c, driver) !== friendlyType(c.dataType, driver) || friendlyType(c.dataType, driver) === "Custom") && <span className={s.raw}>{c.dataType}</span>}
                </span>
                <span className={s.default}>{c.generated ? "Calculated" : describeDefault(c) || <span className={s.none}>—</span>}</span>
                <span className={s.badges}>
                  {c.primaryKey && <span className={s.badge}>Primary key</span>}
                  {!c.nullable && !c.primaryKey && <span className={s.badge}>Required</span>}
                  {uniqueOf(c.name) && <span className={s.badge}>Unique</span>}
                  {link && (
                    <span className={`${s.badge} ${s.linkBadge}`}>
                      <LinkIcon size={11} /> {link.refTable}
                    </span>
                  )}
                </span>
                <span className={s.rowActions}>
                  {editableCol && (
                    <IconButton label={`Edit ${c.name}`} onPress={() => setSheet({ kind: "column", index: i })}>
                      <EditIcon size={14} />
                    </IconButton>
                  )}
                </span>
              </div>
            );
          })}
        </div>
      )}

      {section === "indexes" && (
        <div className={s.list}>
          <p className={s.explain}>Indexes make lookups on these columns faster. A unique index also stops duplicate values.</p>
          {design.indexes.length === 0 && <p className={s.empty}>No indexes on this table yet.</p>}
          {design.indexes.map((x) => (
            <div key={x.name} className={s.item}>
              <span className={s.itemMain}>
                <span className={s.name}>{x.columns.join(", ")}</span>
                {x.unique && <span className={s.badge}>Unique</span>}
              </span>
              <span className={s.itemMeta}>{x.name}</span>
              {!readOnly && (
                <IconButton label={`Remove index ${x.name}`} onPress={() => apply({ ...design, indexes: design.indexes.filter((y) => y !== x) }, "Remove index").catch(() => {})}>
                  <TrashIcon size={14} />
                </IconButton>
              )}
            </div>
          ))}
          {!readOnly && (
            <Button variant="ghost" onPress={() => setSheet({ kind: "index" })}>
              <PlusIcon size={14} /> Add index
            </Button>
          )}
        </div>
      )}

      {section === "links" && (
        <div className={s.list}>
          <p className={s.explain}>A relationship links a column to rows in another table, and keeps those links valid.</p>
          {design.foreignKeys.length === 0 && <p className={s.empty}>This table doesn't link to other tables.</p>}
          {design.foreignKeys.map((k) => (
            <div key={k.name} className={s.item}>
              <span className={s.itemMain}>
                <span className={s.name}>{k.columns.join(", ")}</span>
                <span className={s.arrow}>→</span>
                <span className={s.name}>
                  {k.refTable}.{k.refColumns.join(", ")}
                </span>
              </span>
              <span className={s.itemMeta}>If the {k.refTable} row is deleted: {onDeleteLabel(k.onDelete).toLowerCase()}</span>
              {!readOnly && (
                <IconButton label="Remove relationship" onPress={() => apply({ ...design, foreignKeys: design.foreignKeys.filter((y) => y !== k) }, "Remove relationship").catch(() => {})}>
                  <TrashIcon size={14} />
                </IconButton>
              )}
            </div>
          ))}
          {!readOnly && tables.length > 0 && (
            <Button variant="ghost" onPress={() => setSheet({ kind: "link" })}>
              <PlusIcon size={14} /> Add relationship
            </Button>
          )}
        </div>
      )}

      {sheet?.kind === "column" && (
        <ColumnSheet
          key={sheet.index ?? "new"}
          design={design}
          index={sheet.index}
          driver={driver}
          tables={tables}
          schema={schema ?? snapshot?.defaultSchema ?? null}
          developerMode={developerMode}
          onClose={() => setSheet(null)}
          onSave={(next, title) => apply(next, title).then(() => setSheet(null))}
        />
      )}
      {sheet?.kind === "index" && <IndexSheet design={design} onClose={() => setSheet(null)} onSave={(next) => apply(next, "Add index").then(() => setSheet(null))} />}
      {sheet?.kind === "link" && (
        <LinkSheet design={design} tables={tables} schema={schema ?? snapshot?.defaultSchema ?? null} onClose={() => setSheet(null)} onSave={(next) => apply(next, "Add relationship").then(() => setSheet(null))} />
      )}
      {sheet?.kind === "rename" && <RenameSheet design={design} onClose={() => setSheet(null)} onSave={(next) => apply(next, "Rename table").then(() => setSheet(null))} />}
    </div>
  );
}

// ---- column sheet

function ColumnSheet({
  design,
  index,
  driver,
  tables,
  schema,
  developerMode,
  onClose,
  onSave,
}: {
  design: TableDesign;
  index: number | null;
  driver: DriverInfo | undefined;
  tables: { schema: string; name: string; columns: string[] }[];
  schema: string | null;
  developerMode: boolean;
  onClose(): void;
  onSave(next: TableDesign, title: string): Promise<void>;
}) {
  const existing = index !== null ? design.columns[index] : null;
  const textType = driver?.types.find((t) => t.category === "text")?.sql ?? "text";
  const [col, setCol] = useState<ColumnDesign>(existing ?? { ...blankColumn(), dataType: textType });
  const [unique, setUnique] = useState(existing ? design.indexes.some((x) => x.unique && x.columns.length === 1 && x.columns[0] === existing.name) : false);
  const existingLink = existing ? design.foreignKeys.find((k) => k.columns.length === 1 && k.columns[0] === existing.name) : undefined;
  const [link, setLink] = useState<{ table: string; schema: string | null; column: string; onDelete: FkAction } | null>(
    existingLink ? { table: existingLink.refTable, schema: existingLink.refSchema, column: existingLink.refColumns[0], onDelete: existingLink.onDelete } : null,
  );
  const [literal, setLiteral] = useState(literalOf(existing?.default ?? null));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const types = driver?.types ?? [];
  const known = types.find((t) => t.sql.toLowerCase() === col.dataType.toLowerCase());
  const category = known?.category;
  const numeric = category === "number" || category === "decimal" || category === "boolean";
  const mode = defaultMode(col);
  const mysql = driver?.kind === "mysql";
  const target = link ? tables.find((t) => t.name === link.table && (link.schema ? t.schema === link.schema : t.schema === schema)) : undefined;

  const setMode = (m: DefaultMode) => {
    const patch: Partial<ColumnDesign> = { autoIncrement: m === "auto" };
    if (m === "none" || m === "auto") patch.default = null;
    if (m === "now") patch.default = mysql ? "CURRENT_TIMESTAMP" : "now()";
    if (m === "uuid") patch.default = mysql ? "(uuid())" : "gen_random_uuid()";
    if (m === "value") patch.default = literal === "" ? null : numeric ? literal : sqlString(literal);
    if (m === "custom") patch.default = col.default ?? "";
    setCol((c) => ({ ...c, ...patch }));
  };

  const build = (): TableDesign => {
    const name = col.name.trim();
    const columns = index === null ? [...design.columns, { ...col, name }] : design.columns.map((c, i) => (i === index ? { ...col, name } : c));
    const from = existing?.name ?? name;
    const rename = (cols: string[]) => cols.map((c) => (c === from ? name : c));
    let indexes = design.indexes.map((x) => ({ ...x, columns: rename(x.columns) }));
    const isUnique = (x: IndexDesign) => x.unique && x.columns.length === 1 && x.columns[0] === name;
    if (unique && !indexes.some(isUnique)) indexes = [...indexes, { original: null, name: `${design.name}_${name}_key`, columns: [name], unique: true, isConstraint: false }];
    if (!unique) indexes = indexes.filter((x) => !isUnique(x));
    let foreignKeys = design.foreignKeys.map((k) => ({ ...k, columns: rename(k.columns) }));
    const mine = (k: ForeignKeyDesign) => k.columns.length === 1 && k.columns[0] === name;
    const current = foreignKeys.find(mine);
    if (link) {
      const fk: ForeignKeyDesign = {
        original: current?.original ?? null,
        name: current?.name ?? `${design.name}_${name}_fkey`,
        columns: [name],
        refSchema: link.schema,
        refTable: link.table,
        refColumns: [link.column],
        onDelete: link.onDelete,
        onUpdate: current?.onUpdate ?? "noAction",
      };
      foreignKeys = current ? foreignKeys.map((k) => (k === current ? fk : k)) : [...foreignKeys, fk];
    } else foreignKeys = foreignKeys.filter((k) => !mine(k));
    return { ...design, columns, indexes, foreignKeys };
  };

  const save = async (next: TableDesign, title: string) => {
    setBusy(true);
    setError(null);
    try {
      await onSave(next, title);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Sheet
      isOpen
      onClose={onClose}
      title={existing ? `Edit column` : "Add column"}
      subtitle={existing ? `${design.name}.${existing.name}` : `to ${design.name}`}
      footer={
        <>
          {existing ? (
            <Button
              variant="ghost"
              className={s.dangerText}
              isDisabled={busy}
              onPress={() =>
                save(
                  {
                    ...design,
                    columns: design.columns.filter((_, i) => i !== index),
                    indexes: design.indexes.filter((x) => !x.columns.includes(existing.name)),
                    foreignKeys: design.foreignKeys.filter((k) => !k.columns.includes(existing.name)),
                  },
                  "Delete column",
                )
              }
            >
              <TrashIcon size={14} /> Delete column
            </Button>
          ) : (
            <span />
          )}
          <Button onPress={onClose}>Cancel</Button>
          <Button variant="primary" isDisabled={busy || !col.name.trim() || !col.dataType.trim()} onPress={() => save(build(), existing ? "Save column" : "Add column")}>
            {existing ? "Save" : "Add column"}
          </Button>
        </>
      }
    >
      <div className={f.stack}>
        {error && <div className={f.error} role="alert">{error}</div>}
        <label className={f.field}>
          <span className={f.label}>Name</span>
          <input className={`${f.control} ${f.mono}`} value={col.name} onChange={(e) => setCol({ ...col, name: e.target.value })} placeholder="e.g. email" autoFocus spellCheck={false} />
        </label>

        <label className={f.field}>
          <span className={f.label}>Type</span>
          <select className={f.control} value={known?.sql ?? "__current"} onChange={(e) => e.target.value !== "__current" && setCol({ ...col, dataType: e.target.value })}>
            {!known && <option value="__current">{col.dataType ? `${columnTypeLabel(col, driver)} (${col.dataType})` : "Choose a type"}</option>}
            {types.map((t) => (
              <option key={t.sql} value={t.sql}>
                {t.label}
              </option>
            ))}
          </select>
          {known && <span className={f.help}>{known.hint}</span>}
          {!known && col.enumValues.length > 0 && <span className={f.help}>Allowed values: {col.enumValues.join(", ")}</span>}
        </label>
        {developerMode && (
          <label className={f.field}>
            <span className={f.label}>
              SQL type <span className={f.meta}>advanced</span>
            </span>
            <input className={`${f.control} ${f.mono}`} value={col.dataType} onChange={(e) => setCol({ ...col, dataType: e.target.value })} spellCheck={false} />
          </label>
        )}

        <label className={f.field}>
          <span className={f.label}>Default value</span>
          <select className={f.control} value={mode} onChange={(e) => setMode(e.target.value as DefaultMode)}>
            <option value="none">No default</option>
            <option value="value">A fixed value</option>
            {(category === "dateTime" || category === "date" || mode === "now") && <option value="now">The current time</option>}
            {(category === "number" || mode === "auto") && <option value="auto">Auto-increment (1, 2, 3…)</option>}
            {(category === "identifier" || mode === "uuid") && <option value="uuid">A random UUID</option>}
            {(developerMode || mode === "custom") && <option value="custom">SQL expression</option>}
          </select>
          {mode === "value" && (
            <input
              className={f.control}
              value={literal}
              onChange={(e) => {
                setLiteral(e.target.value);
                setCol({ ...col, default: e.target.value === "" ? null : numeric ? e.target.value : sqlString(e.target.value) });
              }}
              placeholder="Value used when none is given"
            />
          )}
          {mode === "custom" && <input className={`${f.control} ${f.mono}`} value={col.default ?? ""} onChange={(e) => setCol({ ...col, default: e.target.value || null })} spellCheck={false} />}
        </label>

        <div className={f.section}>
          <h4 className={f.sectionTitle}>Rules</h4>
          <div className={f.toggles}>
            <Switch isSelected={!col.nullable} onChange={(v) => setCol({ ...col, nullable: !v })} isDisabled={col.primaryKey}>
              Required
            </Switch>
            <p className={f.toggleHelp}>Every row must have a value.</p>
            <Switch isSelected={unique} onChange={setUnique} isDisabled={col.primaryKey}>
              Unique
            </Switch>
            <p className={f.toggleHelp}>No two rows can have the same value.</p>
            <Switch isSelected={col.primaryKey} onChange={(v) => setCol({ ...col, primaryKey: v, nullable: v ? false : col.nullable })}>
              Primary key
            </Switch>
            <p className={f.toggleHelp}>The value that identifies each row.</p>
          </div>
        </div>

        <div className={f.section}>
          <h4 className={f.sectionTitle}>Link to another table</h4>
          <label className={f.field}>
            <span className={f.label}>Links to</span>
            <select
              className={f.control}
              value={link ? `${link.schema ?? ""}.${link.table}` : ""}
              onChange={(e) => {
                if (!e.target.value) return setLink(null);
                const t = tables.find((t) => `${t.schema === schema ? "" : t.schema}.${t.name}` === e.target.value);
                if (t) setLink({ table: t.name, schema: t.schema === schema ? null : t.schema, column: t.columns.includes("id") ? "id" : t.columns[0], onDelete: link?.onDelete ?? "noAction" });
              }}
            >
              <option value="">Nothing</option>
              {tables
                .filter((t) => t.name !== design.name || t.schema !== schema)
                .map((t) => (
                  <option key={`${t.schema}.${t.name}`} value={`${t.schema === schema ? "" : t.schema}.${t.name}`}>
                    {t.schema === schema ? t.name : `${t.schema}.${t.name}`}
                  </option>
                ))}
            </select>
            <span className={f.help}>Values in this column must be the ID of a row in that table.</span>
          </label>
          {link && target && (
            <div className={f.row}>
              <label className={f.field}>
                <span className={f.label}>Matching column</span>
                <select className={f.control} value={link.column} onChange={(e) => setLink({ ...link, column: e.target.value })}>
                  {target.columns.map((c) => (
                    <option key={c}>{c}</option>
                  ))}
                </select>
              </label>
              <label className={f.field}>
                <span className={f.label}>If that row is deleted</span>
                <select className={f.control} value={link.onDelete} onChange={(e) => setLink({ ...link, onDelete: e.target.value as FkAction })}>
                  {ON_DELETE.map((o) => (
                    <option key={o.value} value={o.value}>
                      {o.label}
                    </option>
                  ))}
                </select>
              </label>
            </div>
          )}
        </div>

        <label className={f.field}>
          <span className={f.label}>
            Description <span className={f.meta}>optional</span>
          </span>
          <textarea className={f.control} rows={2} value={col.comment ?? ""} onChange={(e) => setCol({ ...col, comment: e.target.value || null })} />
        </label>
      </div>
    </Sheet>
  );
}

// ---- index / relationship / rename sheets

function IndexSheet({ design, onClose, onSave }: { design: TableDesign; onClose(): void; onSave(next: TableDesign): Promise<void> }) {
  const [cols, setCols] = useState<string[]>([]);
  const [unique, setUnique] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const save = async () => {
    try {
      await onSave({ ...design, indexes: [...design.indexes, { original: null, name: `${design.name}_${cols.join("_")}_${unique ? "key" : "idx"}`, columns: cols, unique, isConstraint: false }] });
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  return (
    <Sheet
      isOpen
      onClose={onClose}
      title="Add index"
      subtitle={`on ${design.name}`}
      footer={
        <>
          <span />
          <Button onPress={onClose}>Cancel</Button>
          <Button variant="primary" isDisabled={!cols.length} onPress={save}>
            Add index
          </Button>
        </>
      }
    >
      <div className={f.stack}>
        {error && <div className={f.error}>{error}</div>}
        <div className={f.field}>
          <span className={f.label}>Columns</span>
          <span className={f.help}>Pick them in the order you search by them most.</span>
          <div className={s.checkList}>
            {design.columns.map((c) => (
              <label key={c.name}>
                <input type="checkbox" checked={cols.includes(c.name)} onChange={(e) => setCols((cur) => (e.target.checked ? [...cur, c.name] : cur.filter((x) => x !== c.name)))} />
                <span className={s.name}>{c.name}</span>
                {cols.includes(c.name) && <span className={f.meta}>{cols.indexOf(c.name) + 1}</span>}
              </label>
            ))}
          </div>
        </div>
        <Switch isSelected={unique} onChange={setUnique}>
          Unique: no two rows can share these values
        </Switch>
      </div>
    </Sheet>
  );
}

function LinkSheet({
  design,
  tables,
  schema,
  onClose,
  onSave,
}: {
  design: TableDesign;
  tables: { schema: string; name: string; columns: string[] }[];
  schema: string | null;
  onClose(): void;
  onSave(next: TableDesign): Promise<void>;
}) {
  const guess = design.columns.find((c) => /_id$/.test(c.name) && !c.primaryKey) ?? design.columns[0];
  const [column, setColumn] = useState(guess?.name ?? "");
  const guessTable = tables.find((t) => column.replace(/_id$/, "") === t.name.replace(/s$/, "")) ?? tables[0];
  const [table, setTable] = useState(guessTable ? `${guessTable.schema}.${guessTable.name}` : "");
  const target = tables.find((t) => `${t.schema}.${t.name}` === table);
  const [refColumn, setRefColumn] = useState("");
  const [onDelete, setOnDelete] = useState<FkAction>("noAction");
  const [error, setError] = useState<string | null>(null);
  useEffect(() => setRefColumn(target ? (target.columns.includes("id") ? "id" : target.columns[0]) : ""), [target]);

  const save = async () => {
    if (!target) return;
    try {
      await onSave({
        ...design,
        foreignKeys: [
          ...design.foreignKeys,
          { original: null, name: `${design.name}_${column}_fkey`, columns: [column], refSchema: target.schema === schema ? null : target.schema, refTable: target.name, refColumns: [refColumn], onDelete, onUpdate: "noAction" },
        ],
      });
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  return (
    <Sheet
      isOpen
      onClose={onClose}
      title="Add relationship"
      subtitle={`from ${design.name}`}
      footer={
        <>
          <span />
          <Button onPress={onClose}>Cancel</Button>
          <Button variant="primary" isDisabled={!target || !column} onPress={save}>
            Add relationship
          </Button>
        </>
      }
    >
      <div className={f.stack}>
        {error && <div className={f.error}>{error}</div>}
        <label className={f.field}>
          <span className={f.label}>Column in {design.name}</span>
          <select className={f.control} value={column} onChange={(e) => setColumn(e.target.value)}>
            {design.columns.map((c) => (
              <option key={c.name}>{c.name}</option>
            ))}
          </select>
        </label>
        <div className={f.row}>
          <label className={f.field}>
            <span className={f.label}>Links to table</span>
            <select className={f.control} value={table} onChange={(e) => setTable(e.target.value)}>
              {tables.map((t) => (
                <option key={`${t.schema}.${t.name}`} value={`${t.schema}.${t.name}`}>
                  {t.schema === schema ? t.name : `${t.schema}.${t.name}`}
                </option>
              ))}
            </select>
          </label>
          <label className={f.field}>
            <span className={f.label}>Column</span>
            <select className={f.control} value={refColumn} onChange={(e) => setRefColumn(e.target.value)}>
              {target?.columns.map((c) => (
                <option key={c}>{c}</option>
              ))}
            </select>
          </label>
        </div>
        <label className={f.field}>
          <span className={f.label}>If the linked row is deleted</span>
          <select className={f.control} value={onDelete} onChange={(e) => setOnDelete(e.target.value as FkAction)}>
            {ON_DELETE.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </label>
      </div>
    </Sheet>
  );
}

function RenameSheet({ design, onClose, onSave }: { design: TableDesign; onClose(): void; onSave(next: TableDesign): Promise<void> }) {
  const [name, setName] = useState(design.name);
  const [error, setError] = useState<string | null>(null);
  return (
    <Sheet
      isOpen
      onClose={onClose}
      title="Rename table"
      width={420}
      footer={
        <>
          <span />
          <Button onPress={onClose}>Cancel</Button>
          <Button variant="primary" isDisabled={!name.trim() || name.trim() === design.name} onPress={() => onSave({ ...design, name: name.trim() }).catch((e) => setError(errorMessage(e)))}>
            Rename
          </Button>
        </>
      }
    >
      <div className={f.stack}>
        {error && <div className={f.error}>{error}</div>}
        <label className={f.field}>
          <span className={f.label}>New name</span>
          <input className={`${f.control} ${f.mono}`} value={name} onChange={(e) => setName(e.target.value)} autoFocus spellCheck={false} />
          <span className={f.help}>Queries and apps that use the old name will need to be updated.</span>
        </label>
      </div>
    </Sheet>
  );
}

// =====================================================================================
// New table

type DraftColumn = ColumnDesign & { unique?: boolean; linkTo?: string };

export function CreateTable({
  connection,
  schema,
  snapshot,
  onCreated,
  onReview,
  review,
}: {
  connection: ConnectionConfig;
  schema: string | null;
  snapshot: SchemaSnapshot | undefined;
  onCreated(name: string): void | Promise<void>;
  onReview(r: ReviewRequest): void;
  review: ReactNode;
}) {
  const driver = driverFor(connection, useCatalog((st) => st.drivers));
  const mysql = driver?.kind === "mysql";
  const sql = (label: string, fallback: string) => driver?.types.find((t) => t.label === label)?.sql ?? fallback;
  const tables = useMemo(() => tablesOf(snapshot), [snapshot]);
  const [name, setName] = useState("");
  const [columns, setColumns] = useState<DraftColumn[]>(() => [
    { ...blankColumn("id"), dataType: sql("Big integer", "bigint"), nullable: false, primaryKey: true, autoIncrement: true },
    { ...blankColumn("name"), dataType: sql("Text", "text"), nullable: false },
    { ...blankColumn("created_at"), dataType: sql("Date & time", "timestamp"), nullable: false, default: mysql ? "CURRENT_TIMESTAMP" : "now()" },
  ]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const set = (i: number, patch: Partial<DraftColumn>) => setColumns((cur) => cur.map((c, j) => (j === i ? { ...c, ...patch } : c)));

  const create = async () => {
    setBusy(true);
    setError(null);
    const table = name.trim();
    const design: TableDesign = {
      name: table,
      columns: columns.map(({ unique: _u, linkTo: _l, ...c }) => ({ ...c, name: c.name.trim() })),
      indexes: columns.filter((c) => c.unique && !c.primaryKey).map((c) => ({ original: null, name: `${table}_${c.name}_key`, columns: [c.name], unique: true, isConstraint: false })),
      foreignKeys: columns
        .filter((c) => c.linkTo)
        .map((c) => {
          const t = tables.find((t) => `${t.schema}.${t.name}` === c.linkTo)!;
          return {
            original: null,
            name: `${table}_${c.name}_fkey`,
            columns: [c.name],
            refSchema: t.schema === schema ? null : t.schema,
            refTable: t.name,
            refColumns: [t.columns.includes("id") ? "id" : t.columns[0]],
            onDelete: "noAction" as FkAction,
            onUpdate: "noAction" as FkAction,
          };
        }),
      primaryKeyName: null,
    };
    try {
      await applyDesign({ connection, schema, original: null, next: design, driver, onReview, title: "Create table", done: () => onCreated(table) });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className={s.page}>
      <div className={s.header}>
        <div>
          <h2 className={s.title}>New table</h2>
          <p className={s.meta}>Name it, then list the columns each row should have.</p>
        </div>
      </div>
      <div className={s.createForm}>
        <label className={f.field}>
          <span className={f.label}>Table name</span>
          <input className={`${f.control} ${f.mono}`} value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. customers" autoFocus spellCheck={false} style={{ maxWidth: 360 }} />
        </label>

        {error && <div className={f.error} role="alert">{error}</div>}

        <div className={s.createGrid}>
          <div className={s.createHead}>
            <span>Column</span>
            <span>Type</span>
            <span>Default</span>
            <span title="Every row must have a value">Required</span>
            <span title="No duplicates">Unique</span>
            <span title="Identifies each row">Key</span>
            <span>Links to</span>
            <span />
          </div>
          {columns.map((c, i) => {
            const known = driver?.types.find((t) => t.sql === c.dataType);
            return (
              <div key={i} className={s.createRow}>
                <input className={`${f.control} ${f.mono}`} value={c.name} onChange={(e) => set(i, { name: e.target.value })} placeholder="column_name" spellCheck={false} aria-label="Column name" />
                <select className={f.control} value={known?.sql ?? c.dataType} onChange={(e) => set(i, { dataType: e.target.value })} aria-label="Type">
                  {driver?.types.map((t) => (
                    <option key={t.sql} value={t.sql}>
                      {t.label}
                    </option>
                  ))}
                </select>
                <span className={s.defaultCell}>{describeDefault(c) || <span className={s.none}>—</span>}</span>
                <input type="checkbox" checked={!c.nullable} disabled={c.primaryKey} onChange={(e) => set(i, { nullable: !e.target.checked })} aria-label="Required" />
                <input type="checkbox" checked={!!c.unique} disabled={c.primaryKey} onChange={(e) => set(i, { unique: e.target.checked })} aria-label="Unique" />
                <input
                  type="radio"
                  name="pk"
                  checked={c.primaryKey}
                  onChange={() => setColumns((cur) => cur.map((x, j) => ({ ...x, primaryKey: j === i, nullable: j === i ? false : x.nullable })))}
                  aria-label="Primary key"
                />
                <select className={f.control} value={c.linkTo ?? ""} onChange={(e) => set(i, { linkTo: e.target.value || undefined })} aria-label="Links to">
                  <option value="">—</option>
                  {tables.map((t) => (
                    <option key={`${t.schema}.${t.name}`} value={`${t.schema}.${t.name}`}>
                      {t.name}
                    </option>
                  ))}
                </select>
                <IconButton label="Remove column" onPress={() => setColumns((cur) => cur.filter((_, j) => j !== i))}>
                  <TrashIcon size={14} />
                </IconButton>
              </div>
            );
          })}
        </div>
        <div className={s.createActions}>
          <Button variant="ghost" onPress={() => setColumns((cur) => [...cur, { ...blankColumn(`column_${cur.length + 1}`), dataType: sql("Text", "text") }])}>
            <PlusIcon size={14} /> Add column
          </Button>
          <span className={s.spacer} />
          <Button variant="primary" onPress={create} isDisabled={busy || !name.trim() || !columns.length}>
            Create table
          </Button>
        </div>
      </div>
      {review}
    </div>
  );
}
