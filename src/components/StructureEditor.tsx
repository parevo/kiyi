import { forwardRef, useEffect, useImperativeHandle, useMemo, useState } from "react";
import { errorMessage, ipc } from "../lib/ipc";
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
import { driverFor, friendlyType, useCatalog } from "../state/catalog";
import { useSettings } from "../state/settings";
import { CloseIcon, PlusIcon } from "./icons";
import type { ReviewRequest } from "./ReviewDialog";
import { Button, IconButton, Switch } from "./ui";
import s from "./StructureEditor.module.css";

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
});

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
const sqlString = (v: string) => `'${v.replace(/'/g, "''")}'`;

function starterDesign(driver: DriverInfo | undefined): TableDesign {
  const mysql = driver?.kind === "mysql";
  const sql = (label: string, fallback: string) => driver?.types.find((t) => t.label === label)?.sql ?? fallback;
  return {
    name: "",
    columns: [
      { ...blankColumn("id"), dataType: sql("Büyük tam sayı", "bigint"), nullable: false, primaryKey: true, autoIncrement: true },
      { ...blankColumn("created_at"), dataType: sql("Tarih ve saat", "timestamp"), nullable: false, default: mysql ? "CURRENT_TIMESTAMP" : "now()" },
    ],
    indexes: [],
    foreignKeys: [],
    primaryKeyName: null,
  };
}

// ---- defaults in plain language

type DefaultMode = "none" | "value" | "now" | "auto" | "uuid" | "custom";

function defaultMode(c: ColumnDesign): DefaultMode {
  if (c.autoIncrement) return "auto";
  const d = c.default?.trim();
  if (!d) return "none";
  if (/^(now\(\)|current_timestamp(\(\d*\))?)$/i.test(d)) return "now";
  if (/gen_random_uuid\(\)|^\(?uuid\(\)\)?$/i.test(d)) return "uuid";
  if (/^'([^']|'')*'(::[\w\s]+)?$/.test(d) || /^-?\d+(\.\d+)?$/.test(d) || /^(true|false)$/i.test(d)) return "value";
  return "custom";
}

function literalOf(d: string | null): string {
  if (!d) return "";
  const m = /^'((?:[^']|'')*)'(::[\w\s]+)?$/.exec(d.trim());
  return m ? m[1].replace(/''/g, "'") : d.trim();
}

// ---- plain-language change summary

function summarise(original: TableDesign | null, draft: TableDesign, driver: DriverInfo | undefined) {
  const out: { text: string; danger?: boolean }[] = [];
  const ft = (t: string) => friendlyType(t, driver);
  if (!original) {
    out.push({ text: `"${draft.name}" tablosu ${draft.columns.length} sütunla oluşturulacak` });
    for (const c of draft.columns) out.push({ text: `${c.name}: ${ft(c.dataType)}${c.primaryKey ? ", anahtar" : ""}${!c.nullable ? ", zorunlu" : ""}` });
    for (const f of draft.foreignKeys) out.push({ text: `${f.columns.join(", ")} → ${f.refTable} tablosuna bağlanacak` });
    return out;
  }
  if (draft.name !== original.name) out.push({ text: `Tablonun adı "${original.name}" → "${draft.name}" olacak` });
  const byOriginal = new Map(draft.columns.filter((c) => c.original).map((c) => [c.original!, c]));
  for (const o of original.columns) {
    const c = byOriginal.get(o.name);
    if (!c) {
      out.push({ text: `"${o.name}" sütunu ve içindeki tüm veriler silinecek`, danger: true });
      continue;
    }
    if (c.name !== o.name) out.push({ text: `"${o.name}" sütununun adı "${c.name}" olacak` });
    if (c.dataType !== o.dataType) out.push({ text: `"${c.name}" türü ${ft(o.dataType)} → ${ft(c.dataType)} olacak` });
    if (c.nullable !== o.nullable) out.push({ text: c.nullable ? `"${c.name}" artık boş bırakılabilecek` : `"${c.name}" zorunlu olacak` });
    if (c.default !== o.default || c.autoIncrement !== o.autoIncrement) out.push({ text: `"${c.name}" için varsayılan değer değişecek` });
    if (c.primaryKey !== o.primaryKey) out.push({ text: c.primaryKey ? `"${c.name}" anahtar olacak` : `"${c.name}" artık anahtar olmayacak` });
    if (c.comment !== o.comment) out.push({ text: `"${c.name}" açıklaması güncellenecek` });
  }
  for (const c of draft.columns.filter((c) => !c.original)) out.push({ text: `"${c.name}" sütunu eklenecek (${ft(c.dataType)})` });
  const describeIndex = (i: IndexDesign) => (i.unique ? `${i.columns.join(", ")} benzersiz olacak` : `${i.columns.join(", ")} için arama hızlandırılacak`);
  for (const i of draft.indexes) {
    const o = original.indexes.find((x) => x.name === i.original);
    if (!o || !same(o, i)) out.push({ text: describeIndex(i) });
  }
  for (const o of original.indexes) {
    if (!draft.indexes.some((i) => i.original === o.name && same(i, o))) {
      if (!draft.indexes.some((i) => i.original === o.name)) out.push({ text: o.unique ? `${o.columns.join(", ")} artık benzersiz olmak zorunda değil` : `"${o.name}" index'i kaldırılacak` });
    }
  }
  for (const f of draft.foreignKeys) {
    const o = original.foreignKeys.find((x) => x.name === f.original);
    if (!o || !same(o, f)) out.push({ text: `${f.columns.join(", ")} → ${f.refTable} bağlantısı ${o ? "güncellenecek" : "eklenecek"}` });
  }
  for (const o of original.foreignKeys) {
    if (!draft.foreignKeys.some((f) => f.original === o.name)) out.push({ text: `${o.columns.join(", ")} → ${o.refTable} bağlantısı kaldırılacak` });
  }
  return out;
}

export interface StructureHandle {
  save(): void;
  discard(): void;
}

export interface StructureStatus {
  changes: number;
}

interface Props {
  connection: ConnectionConfig;
  schema: string | null;
  /** `null` while creating a new table. */
  details: TableDetails | null;
  snapshot: SchemaSnapshot | undefined;
  active: boolean;
  onStatus(status: StructureStatus): void;
  onReview(req: ReviewRequest): void;
  onApplied(name: string): void;
}

export const StructureEditor = forwardRef<StructureHandle, Props>(function StructureEditor(
  { connection, schema, details, snapshot, active, onStatus, onReview, onApplied },
  ref,
) {
  const drivers = useCatalog((st) => st.drivers);
  const driver = driverFor(connection, drivers);
  const developerMode = useSettings((st) => st.developerMode);
  const inspectorOpen = useSettings((st) => st.inspectorOpen);

  const original = details?.design ?? null;
  const [draft, setDraft] = useState<TableDesign>(() => original ?? starterDesign(driver));
  const [selected, setSelected] = useState<number | null>(original ? null : 0);
  const [error, setError] = useState<string | null>(null);
  const [advanced, setAdvanced] = useState(developerMode);

  useEffect(() => {
    setDraft(original ?? starterDesign(driver));
    setSelected(original ? null : 0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [original]);

  const readOnly = connection.readOnly || !!details?.isView;
  const creating = original === null;
  const summary = useMemo(() => summarise(original, draft, driver), [original, draft, driver]);
  const changes = creating ? 0 : summary.length;
  useEffect(() => onStatus({ changes }), [changes, onStatus]);

  const originalCols = useMemo(() => new Map((original?.columns ?? []).map((c) => [c.name, c])), [original]);
  const removedCols = (original?.columns ?? []).filter((c) => !draft.columns.some((d) => d.original === c.name));
  const colState = (c: ColumnDesign) => (c.original === null ? "new" : same(c, originalCols.get(c.original)) ? undefined : "changed");

  const tables = useMemo(() => {
    const out: { schema: string; name: string; columns: string[] }[] = [];
    for (const sc of snapshot?.schemas ?? []) for (const t of sc.tables) out.push({ schema: sc.name, name: t.name, columns: t.columns.map((c) => c.name) });
    return out;
  }, [snapshot]);
  const sameSchema = (refSchema: string | null) => refSchema === null || refSchema === (schema ?? snapshot?.defaultSchema);

  // ---- mutations

  const setCol = (i: number, patch: Partial<ColumnDesign>) =>
    setDraft((d) => {
      const columns = d.columns.map((c, j) => (j === i ? { ...c, ...patch } : c));
      if (patch.name !== undefined) {
        // Keep indexes and links pointing at a renamed column.
        const from = d.columns[i].name;
        const rename = (cols: string[]) => cols.map((c) => (c === from ? patch.name! : c));
        return {
          ...d,
          columns,
          indexes: d.indexes.map((x) => ({ ...x, columns: rename(x.columns) })),
          foreignKeys: d.foreignKeys.map((f) => ({ ...f, columns: rename(f.columns) })),
        };
      }
      return { ...d, columns };
    });

  const removeCol = (i: number) => {
    setDraft((d) => {
      const name = d.columns[i].name;
      return {
        ...d,
        columns: d.columns.filter((_, j) => j !== i),
        indexes: d.indexes.map((x) => ({ ...x, columns: x.columns.filter((c) => c !== name) })).filter((x) => x.columns.length),
        foreignKeys: d.foreignKeys.filter((f) => !f.columns.includes(name)),
      };
    });
    setSelected(null);
  };

  const addCol = () => {
    const text = driver?.types.find((t) => t.category === "text")?.sql ?? "text";
    setDraft((d) => ({ ...d, columns: [...d.columns, { ...blankColumn(`yeni_sutun_${d.columns.length + 1}`), dataType: text }] }));
    setSelected(draft.columns.length);
  };

  const isUnique = (name: string) => draft.indexes.some((x) => x.unique && x.columns.length === 1 && x.columns[0] === name);
  const setUnique = (name: string, on: boolean) =>
    setDraft((d) => ({
      ...d,
      indexes: on
        ? [...d.indexes, { original: null, name: `${d.name || "tablo"}_${name}_key`, columns: [name], unique: true, isConstraint: false }]
        : d.indexes.filter((x) => !(x.unique && x.columns.length === 1 && x.columns[0] === name)),
    }));

  const linkOf = (name: string) => draft.foreignKeys.find((f) => f.columns.length === 1 && f.columns[0] === name);
  const setLink = (name: string, patch: Partial<ForeignKeyDesign> | null) =>
    setDraft((d) => {
      const existing = d.foreignKeys.find((f) => f.columns.length === 1 && f.columns[0] === name);
      if (patch === null) return { ...d, foreignKeys: d.foreignKeys.filter((f) => f !== existing) };
      if (existing) return { ...d, foreignKeys: d.foreignKeys.map((f) => (f === existing ? { ...f, ...patch } : f)) };
      return {
        ...d,
        foreignKeys: [
          ...d.foreignKeys,
          { original: null, name: `${d.name || "tablo"}_${name}_fkey`, columns: [name], refSchema: null, refTable: "", refColumns: [], onDelete: "noAction", onUpdate: "noAction", ...patch },
        ],
      };
    });

  // ---- apply

  const save = async () => {
    if (readOnly || (!creating && changes === 0)) return;
    setError(null);
    try {
      const statements = await ipc.planTable(connection.id, schema, original, draft);
      if (!statements.length) return;
      onReview({
        title: creating ? "Tabloyu oluştur" : "Yapı değişikliklerini uygula",
        subtitle: creating ? draft.name : original!.name,
        summary,
        statements,
        action: creating ? "Oluştur" : "Uygula",
        confirmWord: original?.name,
        nonTransactional: driver?.kind === "mysql",
        run: async () => {
          await ipc.executeScript(connection.id, statements, "schema");
          onApplied(draft.name);
        },
      });
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const discard = () => {
    setDraft(original ?? starterDesign(driver));
    setSelected(null);
  };
  useImperativeHandle(ref, () => ({ save, discard }));

  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey && e.key.toLowerCase() === "s") {
        e.preventDefault();
        save();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const col = selected !== null ? draft.columns[selected] : undefined;

  return (
    <div className={s.layout}>
      <div className={s.main}>
        <input
          className={s.title}
          value={draft.name}
          onChange={(e) => setDraft((d) => ({ ...d, name: e.target.value }))}
          placeholder="Tablo adı"
          disabled={readOnly}
          autoFocus={creating}
          spellCheck={false}
          aria-label="Tablo adı"
        />
        {details?.isView && <p className={s.note}>Bu bir görünüm (view); yapısı buradan değiştirilemez.</p>}
        {connection.readOnly && !details?.isView && <p className={s.note}>Bağlantı salt okunur; yapı değiştirilemez.</p>}
        {error && (
          <div className={s.error} role="alert">
            {error}
          </div>
        )}

        <h3 className={s.heading}>Sütunlar</h3>
        <ul className={s.columns}>
          {draft.columns.map((c, i) => {
            const link = linkOf(c.name);
            return (
              <li key={i}>
                <button className={s.column} data-selected={selected === i || undefined} data-state={colState(c)} onClick={() => setSelected(i)}>
                  <span className={s.colName}>{c.name || "adsız"}</span>
                  <span className={s.colType}>{friendlyType(c.dataType, driver)}</span>
                  {developerMode && <span className={s.colRaw}>{c.dataType}</span>}
                  <span className={s.badges}>
                    {c.primaryKey && <span className={s.badge}>Anahtar</span>}
                    {!c.nullable && !c.primaryKey && <span className={s.badge}>Zorunlu</span>}
                    {isUnique(c.name) && <span className={s.badge}>Benzersiz</span>}
                    {c.autoIncrement && <span className={s.badge}>Otomatik</span>}
                    {link && <span className={`${s.badge} ${s.linkBadge}`}>→ {link.refTable || "?"}</span>}
                    {c.generated && <span className={s.badge}>Hesaplanan</span>}
                  </span>
                </button>
              </li>
            );
          })}
          {removedCols.map((c) => (
            <li key={`removed-${c.name}`} className={s.removedRow}>
              <span className={s.colName}>{c.name}</span>
              <span className={s.note}>silinecek</span>
              <Button variant="ghost" onPress={() => setDraft((d) => ({ ...d, columns: [...d.columns, c] }))}>
                Geri al
              </Button>
            </li>
          ))}
        </ul>
        {!readOnly && (
          <Button variant="ghost" onPress={addCol}>
            <PlusIcon size={14} /> Sütun ekle
          </Button>
        )}

        <button className={s.disclosure} onClick={() => setAdvanced((v) => !v)} aria-expanded={advanced}>
          {advanced ? "▾" : "▸"} Gelişmiş: index'ler ve çok sütunlu bağlantılar
        </button>
        {advanced && <Advanced draft={draft} setDraft={setDraft} readOnly={readOnly} tables={tables} sameSchema={sameSchema} />}

        {creating && !readOnly && (
          <div className={s.createBar}>
            <Button variant="primary" onPress={save} isDisabled={!draft.name.trim()}>
              Tabloyu oluştur <span className={s.kbd}>⌘S</span>
            </Button>
          </div>
        )}
      </div>

      {inspectorOpen && (
        <aside className={s.inspector}>
          {col && selected !== null ? (
            <ColumnForm
              key={selected}
              column={col}
              driver={driver}
              developerMode={developerMode}
              readOnly={readOnly}
              unique={isUnique(col.name)}
              link={linkOf(col.name)}
              tables={tables}
              sameSchema={sameSchema}
              onChange={(p) => setCol(selected, p)}
              onUnique={(on) => setUnique(col.name, on)}
              onLink={(p) => setLink(col.name, p)}
              onRemove={() => removeCol(selected)}
            />
          ) : (
            <div className={s.placeholder}>
              <p>Ayarlarını görmek için bir sütun seç.</p>
              {!creating && details && (
                <dl className={s.facts}>
                  <dt>Sütun</dt>
                  <dd>{draft.columns.length}</dd>
                  <dt>Bağlantı</dt>
                  <dd>{draft.foreignKeys.length}</dd>
                  <dt>Index</dt>
                  <dd>{draft.indexes.length}</dd>
                </dl>
              )}
            </div>
          )}
        </aside>
      )}
    </div>
  );
});

function ColumnForm({
  column: c,
  driver,
  developerMode,
  readOnly,
  unique,
  link,
  tables,
  sameSchema,
  onChange,
  onUnique,
  onLink,
  onRemove,
}: {
  column: ColumnDesign;
  driver: DriverInfo | undefined;
  developerMode: boolean;
  readOnly: boolean;
  unique: boolean;
  link: ForeignKeyDesign | undefined;
  tables: { schema: string; name: string; columns: string[] }[];
  sameSchema(schema: string | null): boolean;
  onChange(p: Partial<ColumnDesign>): void;
  onUnique(on: boolean): void;
  onLink(p: Partial<ForeignKeyDesign> | null): void;
  onRemove(): void;
}) {
  const mysql = driver?.kind === "mysql";
  const types = driver?.types ?? [];
  const known = types.some((t) => t.sql.toLowerCase() === c.dataType.toLowerCase());
  const mode = defaultMode(c);
  const [literal, setLiteral] = useState(literalOf(c.default));
  const disabled = readOnly || c.generated;
  const category = types.find((t) => t.sql === c.dataType)?.category;
  const numeric = category === "number" || category === "decimal" || category === "boolean";

  const setMode = (m: DefaultMode) => {
    const patch: Partial<ColumnDesign> = { autoIncrement: m === "auto" };
    if (m === "none" || m === "auto") patch.default = null;
    if (m === "now") patch.default = mysql ? "CURRENT_TIMESTAMP" : "now()";
    if (m === "uuid") patch.default = mysql ? "(uuid())" : "gen_random_uuid()";
    if (m === "value") patch.default = literal === "" ? null : numeric ? literal : sqlString(literal);
    if (m === "custom") patch.default = c.default ?? "";
    onChange(patch);
  };

  const target = link ? tables.find((t) => t.name === link.refTable && (link.refSchema ? t.schema === link.refSchema : sameSchema(t.schema))) : undefined;

  return (
    <div className={s.form}>
      <label className={s.field}>
        <span>Ad</span>
        <input className={s.input} value={c.name} onChange={(e) => onChange({ name: e.target.value })} disabled={readOnly} spellCheck={false} autoFocus={c.original === null} />
      </label>

      <label className={s.field}>
        <span>Tür</span>
        <select className={s.input} value={known ? types.find((t) => t.sql.toLowerCase() === c.dataType.toLowerCase())!.sql : "__current"} onChange={(e) => e.target.value !== "__current" && onChange({ dataType: e.target.value })} disabled={disabled}>
          {!known && <option value="__current">{friendlyType(c.dataType, driver)} ({c.dataType})</option>}
          {types.map((t) => (
            <option key={t.sql} value={t.sql}>
              {t.label}
            </option>
          ))}
        </select>
        <small>{types.find((t) => t.sql === c.dataType)?.hint ?? (developerMode ? "" : "Veritabanındaki mevcut tür korunur.")}</small>
      </label>
      {developerMode && (
        <label className={s.field}>
          <span>SQL türü</span>
          <input className={`${s.input} ${s.mono}`} value={c.dataType} onChange={(e) => onChange({ dataType: e.target.value })} disabled={disabled} spellCheck={false} />
        </label>
      )}

      <div className={s.switches}>
        <Switch isSelected={!c.nullable} onChange={(v) => onChange({ nullable: !v })} isDisabled={disabled || c.primaryKey}>
          Zorunlu alan
        </Switch>
        <Switch isSelected={unique} onChange={onUnique} isDisabled={readOnly}>
          Benzersiz olsun
        </Switch>
        <Switch isSelected={c.primaryKey} onChange={(v) => onChange({ primaryKey: v, nullable: v ? false : c.nullable })} isDisabled={disabled}>
          Anahtar (her satırı tanımlar)
        </Switch>
      </div>

      <label className={s.field}>
        <span>Varsayılan değer</span>
        <select className={s.input} value={mode} onChange={(e) => setMode(e.target.value as DefaultMode)} disabled={disabled}>
          <option value="none">Yok</option>
          <option value="value">Sabit bir değer</option>
          {(category === "dateTime" || category === "date" || mode === "now") && <option value="now">Kayıt anındaki zaman</option>}
          {(category === "number" || mode === "auto") && <option value="auto">Otomatik artan sayı</option>}
          {(category === "identifier" || mode === "uuid") && <option value="uuid">Rastgele kimlik (UUID)</option>}
          {(developerMode || mode === "custom") && <option value="custom">Özel SQL ifadesi</option>}
        </select>
      </label>
      {mode === "value" && (
        <input
          className={s.input}
          value={literal}
          onChange={(e) => {
            setLiteral(e.target.value);
            onChange({ default: e.target.value === "" ? null : numeric ? e.target.value : sqlString(e.target.value) });
          }}
          placeholder="değer"
          disabled={disabled}
        />
      )}
      {mode === "custom" && (
        <input className={`${s.input} ${s.mono}`} value={c.default ?? ""} onChange={(e) => onChange({ default: e.target.value || null })} disabled={disabled} spellCheck={false} />
      )}

      <div className={s.field}>
        <span>Başka bir tabloya bağlı</span>
        <select
          className={s.input}
          value={link ? `${link.refSchema ?? ""}.${link.refTable}` : ""}
          disabled={readOnly}
          onChange={(e) => {
            if (!e.target.value) return onLink(null);
            const t = tables.find((t) => `${sameSchema(t.schema) ? "" : t.schema}.${t.name}` === e.target.value);
            if (t) onLink({ refSchema: sameSchema(t.schema) ? null : t.schema, refTable: t.name, refColumns: [t.columns.includes("id") ? "id" : t.columns[0]] });
          }}
        >
          <option value="">Bağlı değil</option>
          {tables.map((t) => (
            <option key={`${t.schema}.${t.name}`} value={`${sameSchema(t.schema) ? "" : t.schema}.${t.name}`}>
              {sameSchema(t.schema) ? t.name : `${t.schema}.${t.name}`}
            </option>
          ))}
        </select>
        {link && target && (
          <>
            <select className={s.input} value={link.refColumns[0] ?? ""} onChange={(e) => onLink({ refColumns: [e.target.value] })} disabled={readOnly} aria-label="Hedef sütun">
              {target.columns.map((tc) => (
                <option key={tc} value={tc}>
                  {target.name}.{tc}
                </option>
              ))}
            </select>
            <label className={s.field}>
              <span>Bağlı kayıt silinirse</span>
              <select className={s.input} value={link.onDelete} onChange={(e) => onLink({ onDelete: e.target.value as FkAction })} disabled={readOnly}>
                <option value="noAction">Silmeyi engelle</option>
                <option value="cascade">Bu satır da silinsin</option>
                <option value="setNull">Bu alan boşaltılsın</option>
              </select>
            </label>
          </>
        )}
      </div>

      <label className={s.field}>
        <span>Açıklama</span>
        <textarea className={s.input} rows={2} value={c.comment ?? ""} onChange={(e) => onChange({ comment: e.target.value || null })} disabled={readOnly} />
      </label>

      {!readOnly && (
        <Button variant="ghost" className={s.remove} onPress={onRemove}>
          Sütunu sil
        </Button>
      )}
    </div>
  );
}

/** Index and multi-column link editing for people who need it. */
function Advanced({
  draft,
  setDraft,
  readOnly,
  tables,
  sameSchema,
}: {
  draft: TableDesign;
  setDraft: React.Dispatch<React.SetStateAction<TableDesign>>;
  readOnly: boolean;
  tables: { schema: string; name: string; columns: string[] }[];
  sameSchema(schema: string | null): boolean;
}) {
  const names = draft.columns.map((c) => c.name);
  const setIndex = (i: number, p: Partial<IndexDesign>) => setDraft((d) => ({ ...d, indexes: d.indexes.map((x, j) => (j === i ? { ...x, ...p } : x)) }));
  const setFk = (i: number, p: Partial<ForeignKeyDesign>) => setDraft((d) => ({ ...d, foreignKeys: d.foreignKeys.map((x, j) => (j === i ? { ...x, ...p } : x)) }));
  const refCols = (f: ForeignKeyDesign) => tables.find((t) => t.name === f.refTable && (f.refSchema ? t.schema === f.refSchema : sameSchema(t.schema)))?.columns ?? [];

  return (
    <div className={s.advanced}>
      <h4 className={s.subheading}>Index'ler</h4>
      {draft.indexes.length === 0 && <p className={s.note}>Index yok.</p>}
      {draft.indexes.map((x, i) => (
        <div key={i} className={s.line}>
          <input className={`${s.input} ${s.mono}`} style={{ width: 220 }} value={x.name} onChange={(e) => setIndex(i, { name: e.target.value })} disabled={readOnly} spellCheck={false} />
          <ColumnPicker all={names} value={x.columns} onChange={(columns) => setIndex(i, { columns })} disabled={readOnly} />
          <label className={s.inline}>
            <input type="checkbox" checked={x.unique} onChange={(e) => setIndex(i, { unique: e.target.checked })} disabled={readOnly} /> Benzersiz
          </label>
          {!readOnly && (
            <IconButton label="Index'i sil" onPress={() => setDraft((d) => ({ ...d, indexes: d.indexes.filter((_, j) => j !== i) }))}>
              <CloseIcon size={13} />
            </IconButton>
          )}
        </div>
      ))}
      {!readOnly && (
        <Button
          variant="ghost"
          onPress={() =>
            setDraft((d) => ({ ...d, indexes: [...d.indexes, { original: null, name: `${d.name || "tablo"}_${names[0] ?? "x"}_idx`, columns: names[0] ? [names[0]] : [], unique: false, isConstraint: false }] }))
          }
        >
          <PlusIcon size={14} /> Index ekle
        </Button>
      )}

      <h4 className={s.subheading}>Bağlantılar (foreign key)</h4>
      {draft.foreignKeys.length === 0 && <p className={s.note}>Bağlantı yok.</p>}
      {draft.foreignKeys.map((f, i) => (
        <div key={i} className={s.line}>
          <input className={`${s.input} ${s.mono}`} style={{ width: 200 }} value={f.name} onChange={(e) => setFk(i, { name: e.target.value })} disabled={readOnly} spellCheck={false} />
          <ColumnPicker all={names} value={f.columns} onChange={(columns) => setFk(i, { columns })} disabled={readOnly} />
          <span className={s.note}>→ {f.refTable}</span>
          <ColumnPicker all={refCols(f)} value={f.refColumns} onChange={(refColumns) => setFk(i, { refColumns })} disabled={readOnly} />
          {!readOnly && (
            <IconButton label="Bağlantıyı sil" onPress={() => setDraft((d) => ({ ...d, foreignKeys: d.foreignKeys.filter((_, j) => j !== i) }))}>
              <CloseIcon size={13} />
            </IconButton>
          )}
        </div>
      ))}
    </div>
  );
}

/** Ordered multi-column choice shown as removable chips plus an "add" dropdown. */
function ColumnPicker({ all, value, onChange, disabled }: { all: string[]; value: string[]; onChange(v: string[]): void; disabled?: boolean }) {
  const rest = all.filter((c) => !value.includes(c));
  return (
    <span className={s.chips}>
      {value.map((c) => (
        <span key={c} className={s.chip}>
          {c}
          {!disabled && (
            <button className={s.chipX} onClick={() => onChange(value.filter((x) => x !== c))} aria-label={`${c} çıkar`}>
              ×
            </button>
          )}
        </span>
      ))}
      {!disabled && rest.length > 0 && (
        <select className={s.addChip} value="" onChange={(e) => e.target.value && onChange([...value, e.target.value])} aria-label="Sütun ekle">
          <option value="">+</option>
          {rest.map((c) => (
            <option key={c}>{c}</option>
          ))}
        </select>
      )}
    </span>
  );
}
