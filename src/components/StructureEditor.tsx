import { forwardRef, useEffect, useImperativeHandle, useMemo, useState } from "react";
import { errorMessage, ipc } from "../lib/ipc";
import type {
  ColumnDesign,
  ConnectionConfig,
  FkAction,
  ForeignKeyDesign,
  IndexDesign,
  SchemaSnapshot,
  TableDesign,
  TableDetails,
} from "../lib/types";
import { CloseIcon, PlusIcon } from "./icons";
import type { ReviewRequest } from "./ReviewDialog";
import { Button, IconButton } from "./ui";
import s from "./StructureEditor.module.css";

const TYPES: Record<ConnectionConfig["kind"], string[]> = {
  postgres: [
    "bigint", "integer", "smallint", "numeric(12,2)", "real", "double precision",
    "text", "varchar(255)", "char(1)", "boolean", "uuid",
    "timestamptz", "timestamp", "date", "time", "interval",
    "jsonb", "json", "bytea", "inet", "text[]", "integer[]",
  ],
  mysql: [
    "bigint", "int", "smallint", "tinyint", "tinyint(1)", "decimal(12,2)", "float", "double",
    "varchar(255)", "char(36)", "text", "mediumtext", "longtext",
    "datetime", "timestamp", "date", "time", "year",
    "json", "binary(16)", "varbinary(255)", "blob", "enum('a','b')",
  ],
};

const FK_ACTIONS: { value: FkAction; label: string }[] = [
  { value: "noAction", label: "NO ACTION" },
  { value: "restrict", label: "RESTRICT" },
  { value: "cascade", label: "CASCADE" },
  { value: "setNull", label: "SET NULL" },
  { value: "setDefault", label: "SET DEFAULT" },
];

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

function starterDesign(kind: ConnectionConfig["kind"]): TableDesign {
  return {
    name: "",
    columns: [
      { ...blankColumn("id"), dataType: "bigint", nullable: false, primaryKey: true, autoIncrement: true },
      { ...blankColumn("created_at"), dataType: kind === "mysql" ? "datetime" : "timestamptz", nullable: false, default: kind === "mysql" ? "CURRENT_TIMESTAMP" : "now()" },
    ],
    indexes: [],
    foreignKeys: [],
    primaryKeyName: null,
  };
}

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

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
  const original = details?.design ?? null;
  const [draft, setDraft] = useState<TableDesign>(() => original ?? starterDesign(connection.kind));
  const [error, setError] = useState<string | null>(null);
  useEffect(() => setDraft(original ?? starterDesign(connection.kind)), [original, connection.kind]);

  const readOnly = connection.readOnly || !!details?.isView;
  const creating = original === null;

  // ---- change tracking

  const originalCols = useMemo(() => new Map((original?.columns ?? []).map((c) => [c.name, c])), [original]);
  const removedCols = (original?.columns ?? []).filter((c) => !draft.columns.some((d) => d.original === c.name));
  const colState = (c: ColumnDesign) => (c.original === null ? "new" : same(c, originalCols.get(c.original)) ? null : "changed");
  const changes = creating
    ? 1
    : draft.columns.filter(colState).length +
      removedCols.length +
      (draft.name !== original?.name ? 1 : 0) +
      (same(draft.indexes, original?.indexes) ? 0 : 1) +
      (same(draft.foreignKeys, original?.foreignKeys) ? 0 : 1);

  useEffect(() => onStatus({ changes: creating ? 0 : changes }));

  const setCol = (i: number, patch: Partial<ColumnDesign>) =>
    setDraft((d) => {
      const columns = d.columns.map((c, j) => (j === i ? { ...c, ...patch } : c));
      // Keep indexes and FKs pointing at a renamed column.
      if (patch.name !== undefined) {
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

  const removeCol = (i: number) =>
    setDraft((d) => {
      const name = d.columns[i].name;
      return {
        ...d,
        columns: d.columns.filter((_, j) => j !== i),
        indexes: d.indexes.map((x) => ({ ...x, columns: x.columns.filter((c) => c !== name) })).filter((x) => x.columns.length),
        foreignKeys: d.foreignKeys.filter((f) => !f.columns.includes(name)),
      };
    });

  const restoreCol = (c: ColumnDesign) => setDraft((d) => ({ ...d, columns: [...d.columns, c] }));

  const setIndex = (i: number, patch: Partial<IndexDesign>) =>
    setDraft((d) => ({ ...d, indexes: d.indexes.map((x, j) => (j === i ? { ...x, ...patch } : x)) }));
  const setFk = (i: number, patch: Partial<ForeignKeyDesign>) =>
    setDraft((d) => ({ ...d, foreignKeys: d.foreignKeys.map((x, j) => (j === i ? { ...x, ...patch } : x)) }));

  const tables = useMemo(() => {
    const out: { schema: string; name: string; columns: string[] }[] = [];
    for (const sc of snapshot?.schemas ?? []) for (const t of sc.tables) out.push({ schema: sc.name, name: t.name, columns: t.columns.map((c) => c.name) });
    return out;
  }, [snapshot]);
  const multiSchema = (snapshot?.schemas.length ?? 0) > 1;
  const sameSchema = (refSchema: string | null) => refSchema === null || refSchema === (schema ?? snapshot?.defaultSchema);
  const refColumns = (f: ForeignKeyDesign) =>
    tables.find((t) => t.name === f.refTable && (f.refSchema ? t.schema === f.refSchema : sameSchema(t.schema)))?.columns ?? [];

  const addIndex = () => {
    const first = draft.columns[0]?.name ?? "";
    setDraft((d) => ({
      ...d,
      indexes: [...d.indexes, { original: null, name: `${d.name || "tablo"}_${first}_idx`, columns: first ? [first] : [], unique: false, isConstraint: false }],
    }));
  };

  const addFk = () => {
    const col = draft.columns.find((c) => /_id$/.test(c.name)) ?? draft.columns[0];
    const guess = col ? tables.find((t) => col.name.replace(/_id$/, "") === t.name.replace(/s$/, "")) : undefined;
    const target = guess ?? tables[0];
    setDraft((d) => ({
      ...d,
      foreignKeys: [
        ...d.foreignKeys,
        {
          original: null,
          name: `${d.name || "tablo"}_${col?.name ?? "col"}_fkey`,
          columns: col ? [col.name] : [],
          refSchema: target && !sameSchema(target.schema) ? target.schema : null,
          refTable: target?.name ?? "",
          refColumns: target ? [target.columns.includes("id") ? "id" : target.columns[0]] : [],
          onDelete: "noAction",
          onUpdate: "noAction",
        },
      ],
    }));
  };

  // ---- apply

  const save = async () => {
    if (readOnly || (!creating && changes === 0)) return;
    setError(null);
    try {
      const statements = await ipc.planTable(connection.id, schema, original, draft);
      if (!statements.length) return;
      onReview({
        title: creating ? `${draft.name} tablosunu oluştur` : `${original!.name} yapısını değiştir`,
        subtitle: creating ? undefined : `${statements.length} adım`,
        statements,
        action: creating ? "Tabloyu oluştur" : "Değişiklikleri uygula",
        confirmWord: original?.name,
        nonTransactional: connection.kind === "mysql",
        run: async () => {
          await ipc.executeScript(connection.id, statements, "schema");
          onApplied(draft.name);
        },
      });
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const discard = () => setDraft(original ?? starterDesign(connection.kind));
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

  const typesId = `types-${connection.kind}`;

  return (
    <div className={s.editor}>
      <datalist id={typesId}>
        {TYPES[connection.kind].map((t) => (
          <option key={t} value={t} />
        ))}
      </datalist>

      <div className={s.nameRow}>
        <label className={s.nameLabel}>
          Tablo adı
          <input
            className={s.nameInput}
            value={draft.name}
            onChange={(e) => setDraft((d) => ({ ...d, name: e.target.value }))}
            placeholder="ornek_tablo"
            disabled={readOnly}
            autoFocus={creating}
            spellCheck={false}
          />
        </label>
        {details?.isView && <span className={s.note}>View'ların yapısı buradan değiştirilemez.</span>}
        {connection.readOnly && !details?.isView && <span className={s.note}>Salt okunur bağlantı.</span>}
      </div>

      {error && (
        <div className={s.error} role="alert">
          {error}
        </div>
      )}

      <section className={s.section}>
        <h3 className={s.heading}>Sütunlar</h3>
        <table className={s.table}>
          <thead>
            <tr>
              <th style={{ width: "22%" }}>Ad</th>
              <th style={{ width: "18%" }}>Tip</th>
              <th className={s.check} title="NULL olabilir">Null</th>
              <th style={{ width: "18%" }}>Varsayılan</th>
              <th className={s.check} title="Primary key">PK</th>
              <th className={s.check} title={connection.kind === "mysql" ? "AUTO_INCREMENT" : "Identity"}>Oto</th>
              <th>Yorum</th>
              <th className={s.check} />
            </tr>
          </thead>
          <tbody>
            {draft.columns.map((c, i) => (
              <tr key={i} data-state={colState(c) ?? undefined}>
                <td>
                  <input className={s.cell} value={c.name} onChange={(e) => setCol(i, { name: e.target.value })} disabled={readOnly} spellCheck={false} placeholder="sutun_adi" autoFocus={c.original === null && c.name === ""} />
                </td>
                <td>
                  <input className={`${s.cell} ${s.mono}`} value={c.dataType} list={typesId} onChange={(e) => setCol(i, { dataType: e.target.value })} disabled={readOnly || c.generated} spellCheck={false} placeholder="tip" />
                </td>
                <td className={s.check}>
                  <input type="checkbox" checked={c.nullable} onChange={(e) => setCol(i, { nullable: e.target.checked })} disabled={readOnly || c.primaryKey || c.generated} />
                </td>
                <td>
                  <input
                    className={`${s.cell} ${s.mono}`}
                    value={c.default ?? ""}
                    onChange={(e) => setCol(i, { default: e.target.value === "" ? null : e.target.value })}
                    disabled={readOnly || c.autoIncrement || c.generated}
                    placeholder={c.autoIncrement ? "otomatik" : c.generated ? "hesaplanan" : "yok"}
                    title="SQL ifadesi: 0, 'metin', now()"
                    spellCheck={false}
                  />
                </td>
                <td className={s.check}>
                  <input type="checkbox" checked={c.primaryKey} onChange={(e) => setCol(i, { primaryKey: e.target.checked, nullable: e.target.checked ? false : c.nullable })} disabled={readOnly || c.generated} />
                </td>
                <td className={s.check}>
                  <input type="checkbox" checked={c.autoIncrement} onChange={(e) => setCol(i, { autoIncrement: e.target.checked, default: e.target.checked ? null : c.default })} disabled={readOnly || c.generated} />
                </td>
                <td>
                  <input className={s.cell} value={c.comment ?? ""} onChange={(e) => setCol(i, { comment: e.target.value || null })} disabled={readOnly} />
                </td>
                <td className={s.check}>
                  {!readOnly && (
                    <IconButton label={`${c.name || "sütunu"} sil`} onPress={() => removeCol(i)}>
                      <CloseIcon size={13} />
                    </IconButton>
                  )}
                </td>
              </tr>
            ))}
            {removedCols.map((c) => (
              <tr key={`removed-${c.name}`} data-state="removed">
                <td colSpan={7}>
                  <span className={s.removed}>{c.name}</span> <span className={s.note}>silinecek</span>
                </td>
                <td className={s.check}>
                  <Button variant="ghost" onPress={() => restoreCol(c)}>
                    Geri al
                  </Button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {!readOnly && (
          <Button variant="ghost" onPress={() => setDraft((d) => ({ ...d, columns: [...d.columns, blankColumn()] }))}>
            <PlusIcon size={14} /> Sütun ekle
          </Button>
        )}
      </section>

      <section className={s.section}>
        <h3 className={s.heading}>Index'ler</h3>
        {draft.indexes.length === 0 && <p className={s.empty}>Index yok.</p>}
        {draft.indexes.map((x, i) => (
          <div key={i} className={s.line} data-state={x.original === null ? "new" : same(x, original?.indexes.find((o) => o.name === x.original)) ? undefined : "changed"}>
            <input className={`${s.cell} ${s.mono}`} style={{ width: 220 }} value={x.name} onChange={(e) => setIndex(i, { name: e.target.value })} disabled={readOnly} spellCheck={false} />
            <ColumnPicker all={draft.columns.map((c) => c.name)} value={x.columns} onChange={(columns) => setIndex(i, { columns })} disabled={readOnly} />
            <label className={s.inline}>
              <input type="checkbox" checked={x.unique} onChange={(e) => setIndex(i, { unique: e.target.checked })} disabled={readOnly} /> Unique
            </label>
            {!readOnly && (
              <IconButton label="Index'i sil" onPress={() => setDraft((d) => ({ ...d, indexes: d.indexes.filter((_, j) => j !== i) }))}>
                <CloseIcon size={13} />
              </IconButton>
            )}
          </div>
        ))}
        {!readOnly && (
          <Button variant="ghost" onPress={addIndex}>
            <PlusIcon size={14} /> Index ekle
          </Button>
        )}
      </section>

      <section className={s.section}>
        <h3 className={s.heading}>Foreign key'ler</h3>
        {draft.foreignKeys.length === 0 && <p className={s.empty}>Foreign key yok.</p>}
        {draft.foreignKeys.map((f, i) => (
          <div key={i} className={s.line} data-state={f.original === null ? "new" : same(f, original?.foreignKeys.find((o) => o.name === f.original)) ? undefined : "changed"}>
            <input className={`${s.cell} ${s.mono}`} style={{ width: 200 }} value={f.name} onChange={(e) => setFk(i, { name: e.target.value })} disabled={readOnly} spellCheck={false} />
            <ColumnPicker all={draft.columns.map((c) => c.name)} value={f.columns} onChange={(columns) => setFk(i, { columns })} disabled={readOnly} />
            <span className={s.arrow}>→</span>
            <select
              className={s.select}
              value={`${f.refSchema ?? ""}.${f.refTable}`}
              disabled={readOnly}
              onChange={(e) => {
                const t = tables.find((t) => `${sameSchema(t.schema) ? "" : t.schema}.${t.name}` === e.target.value);
                if (t) setFk(i, { refSchema: sameSchema(t.schema) ? null : t.schema, refTable: t.name, refColumns: [t.columns.includes("id") ? "id" : t.columns[0]] });
              }}
            >
              {!tables.some((t) => t.name === f.refTable) && <option value={`${f.refSchema ?? ""}.${f.refTable}`}>{f.refTable}</option>}
              {tables.map((t) => (
                <option key={`${t.schema}.${t.name}`} value={`${sameSchema(t.schema) ? "" : t.schema}.${t.name}`}>
                  {multiSchema ? `${t.schema}.${t.name}` : t.name}
                </option>
              ))}
            </select>
            <ColumnPicker all={refColumns(f)} value={f.refColumns} onChange={(refColumns) => setFk(i, { refColumns })} disabled={readOnly} />
            <label className={s.inline}>
              silinince
              <select className={s.select} value={f.onDelete} onChange={(e) => setFk(i, { onDelete: e.target.value as FkAction })} disabled={readOnly}>
                {FK_ACTIONS.map((a) => (
                  <option key={a.value} value={a.value}>
                    {a.label}
                  </option>
                ))}
              </select>
            </label>
            {!readOnly && (
              <IconButton label="Foreign key'i sil" onPress={() => setDraft((d) => ({ ...d, foreignKeys: d.foreignKeys.filter((_, j) => j !== i) }))}>
                <CloseIcon size={13} />
              </IconButton>
            )}
          </div>
        ))}
        {!readOnly && tables.length > 0 && (
          <Button variant="ghost" onPress={addFk}>
            <PlusIcon size={14} /> Foreign key ekle
          </Button>
        )}
      </section>

      {creating && !readOnly && (
        <div className={s.createBar}>
          <Button variant="primary" onPress={save} isDisabled={!draft.name.trim()}>
            Önizle ve oluştur <span className={s.kbd}>⌘S</span>
          </Button>
        </div>
      )}
    </div>
  );
});

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
