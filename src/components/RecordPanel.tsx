import { useEffect, useState } from "react";
import type { Cell, ColumnDesign, ColumnMeta, DriverInfo, ForeignKeyDesign, TableDetails } from "../lib/types";
import { CATEGORY_LABEL, categorise, friendlyType } from "../state/catalog";
import { ChevronIcon } from "./icons";
import s from "./RecordPanel.module.css";

const fmt = new Intl.NumberFormat("tr-TR");

interface Props {
  details: TableDetails;
  driver: DriverInfo | undefined;
  columns: ColumnMeta[];
  /** Index of the selected row, or null. */
  row: number | null;
  rowCount: number;
  total: number | null;
  value(col: number, row: number): Cell | undefined;
  edited(col: number, row: number): boolean;
  writable(col: number, row: number): boolean;
  onChange(col: number, row: number, value: Cell): void;
  onFollow(fk: ForeignKeyDesign, value: string): void;
  onMove(row: number): void;
  onEditStructure(): void;
}

/** Right-hand panel: the selected row as a form, or a summary of the table. */
export function RecordPanel(props: Props) {
  const { details, driver, columns, row } = props;
  const design = details.design;

  if (row === null || row >= props.rowCount) {
    const fks = design.foreignKeys;
    return (
      <div className={s.panel}>
        <div className={s.summary}>
          <h3 className={s.title}>{design.name}</h3>
          <p className={s.sub}>
            {props.total !== null ? `${fmt.format(props.total)} kayıt` : details.rowEstimate !== null ? `~${fmt.format(details.rowEstimate)} kayıt` : ""}
            {" · "}
            {design.columns.length} alan
          </p>
          <p className={s.hint}>Ayrıntılarını görmek ve düzenlemek için bir satır seç.</p>
        </div>
        <h4 className={s.heading}>Alanlar</h4>
        <ul className={s.fieldList}>
          {design.columns.map((c) => (
            <li key={c.name}>
              <span className={s.mono}>{c.name}</span>
              <span className={s.muted}>{friendlyType(c.dataType, driver)}</span>
            </li>
          ))}
        </ul>
        {fks.length > 0 && (
          <>
            <h4 className={s.heading}>Bağlı olduğu tablolar</h4>
            <ul className={s.fieldList}>
              {fks.map((f) => (
                <li key={f.name}>
                  <span className={s.mono}>{f.columns.join(", ")}</span>
                  <span className={s.muted}>→ {f.refTable}</span>
                </li>
              ))}
            </ul>
          </>
        )}
        {!details.isView && (
          <button className={s.textButton} onClick={props.onEditStructure}>
            Yapıyı düzenle →
          </button>
        )}
      </div>
    );
  }

  const designOf = (name: string) => design.columns.find((c) => c.name === name);
  const linkOf = (name: string) => design.foreignKeys.find((f) => f.columns.length === 1 && f.columns[0] === name);

  return (
    <div className={s.panel}>
      <div className={s.recordHeader}>
        <span className={s.title}>Kayıt {fmt.format(row + 1)}</span>
        <span className={s.nav}>
          <button aria-label="Önceki kayıt" disabled={row === 0} onClick={() => props.onMove(row - 1)}>
            <ChevronIcon size={14} style={{ transform: "rotate(180deg)" }} />
          </button>
          <button aria-label="Sonraki kayıt" disabled={row >= props.rowCount - 1} onClick={() => props.onMove(row + 1)}>
            <ChevronIcon size={14} />
          </button>
        </span>
      </div>
      <div className={s.fields}>
        {columns.map((meta, col) => (
          <FieldEditor
            key={`${row}:${meta.name}`}
            name={meta.name}
            design={designOf(meta.name)}
            category={categorise(designOf(meta.name)?.dataType ?? meta.typeName, driver)}
            typeLabel={designOf(meta.name) ? friendlyType(designOf(meta.name)!.dataType, driver) : meta.typeName}
            value={props.value(col, row)}
            edited={props.edited(col, row)}
            writable={props.writable(col, row)}
            link={linkOf(meta.name)}
            onChange={(v) => props.onChange(col, row, v)}
            onFollow={props.onFollow}
          />
        ))}
      </div>
    </div>
  );
}

function prettyJson(v: string) {
  try {
    return JSON.stringify(JSON.parse(v), null, 2);
  } catch {
    return v;
  }
}

function FieldEditor({
  name,
  design,
  category,
  typeLabel,
  value,
  edited,
  writable,
  link,
  onChange,
  onFollow,
}: {
  name: string;
  design: ColumnDesign | undefined;
  category: ReturnType<typeof categorise>;
  typeLabel: string;
  value: Cell | undefined;
  edited: boolean;
  writable: boolean;
  link: ForeignKeyDesign | undefined;
  onChange(v: Cell): void;
  onFollow(fk: ForeignKeyDesign, value: string): void;
}) {
  const isJson = category === "json";
  const shown = value === null || value === undefined ? "" : isJson ? prettyJson(value) : value;
  const [draft, setDraft] = useState(shown);
  const [jsonError, setJsonError] = useState(false);
  useEffect(() => setDraft(shown), [shown]);

  const nullable = design?.nullable ?? true;
  const isNull = value === null;
  const isDefault = value === undefined;

  const commit = () => {
    if (draft === shown) return;
    if (isJson && draft.trim()) {
      try {
        // Store compact JSON; formatting is only for reading.
        onChange(JSON.stringify(JSON.parse(draft)));
        setJsonError(false);
      } catch {
        setJsonError(true);
      }
      return;
    }
    onChange(draft);
  };

  let input: React.ReactNode;
  if (category === "boolean" && design?.dataType.toLowerCase() !== "tinyint(1)") {
    input = (
      <div className={s.segmented} role="radiogroup" aria-label={name}>
        {(["true", "false"] as const).map((v) => (
          <button key={v} role="radio" aria-checked={value === v} disabled={!writable} onClick={() => onChange(v)}>
            {v === "true" ? "Evet" : "Hayır"}
          </button>
        ))}
      </div>
    );
  } else if (isJson || (category === "text" && (shown.length > 50 || shown.includes("\n")))) {
    input = (
      <textarea
        className={`${s.input} ${isJson ? s.mono : ""}`}
        data-invalid={jsonError || undefined}
        rows={Math.min(12, Math.max(3, draft.split("\n").length))}
        value={draft}
        placeholder={isNull ? "Boş" : isDefault ? "Varsayılan" : ""}
        readOnly={!writable}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        spellCheck={!isJson}
      />
    );
  } else {
    input = (
      <input
        className={`${s.input} ${category === "number" || category === "decimal" || category === "identifier" ? s.mono : ""}`}
        value={draft}
        inputMode={category === "number" || category === "decimal" ? "decimal" : undefined}
        placeholder={isNull ? "Boş" : isDefault ? "Varsayılan" : ""}
        readOnly={!writable || category === "binary"}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") (e.target as HTMLInputElement).blur();
          if (e.key === "Escape") setDraft(shown);
        }}
        spellCheck={false}
      />
    );
  }

  return (
    <div className={s.field} data-edited={edited || undefined}>
      <div className={s.label}>
        <span className={s.name}>{name}</span>
        <span className={s.type}>{typeLabel || CATEGORY_LABEL[category]}</span>
        {design?.primaryKey && <span className={s.tag}>Anahtar</span>}
        {writable && nullable && !isNull && (
          <button className={s.nullButton} onClick={() => onChange(null)} title="Değeri boşalt (NULL)">
            Boşalt
          </button>
        )}
      </div>
      {input}
      {jsonError && <span className={s.error}>Geçerli bir JSON değil.</span>}
      {link && value && (
        <button className={s.textButton} onClick={() => onFollow(link, value)}>
          {link.refTable} kaydını aç →
        </button>
      )}
    </div>
  );
}
