import type { Cell, ColumnMeta, DriverInfo, ForeignKeyDesign, TableDetails } from "../lib/types";
import { useState } from "react";
import { categorise, columnTypeLabel } from "../state/catalog";
import { FieldInput } from "./FieldInput";
import { ValueViewer, type ViewedValue } from "./ValueViewer";
import { ArrowIcon, ChevronIcon, ChevronLeftIcon, KeyIcon, LinkIcon } from "./icons";
import s from "./RecordPanel.module.css";

const fmt = new Intl.NumberFormat("en-US");

interface Props {
  details: TableDetails;
  driver: DriverInfo | undefined;
  columns: ColumnMeta[];
  row: number | null;
  rowCount: number;
  total: number | null;
  value(col: number, row: number): Cell;
  edited(col: number, row: number): boolean;
  writable(col: number): boolean;
  onChange(col: number, row: number, value: Cell): void;
  onFollow(fk: ForeignKeyDesign, value: string): void;
  onMove(row: number): void;
  onEditStructure(): void;
  developerMode: boolean;
}

/** The right-hand panel: the selected row as a form, or an overview of the table. */
export function RecordPanel(props: Props) {
  const { details, driver, columns, row } = props;
  const design = details.design;
  const [viewed, setViewed] = useState<ViewedValue | null>(null);

  if (row === null || row >= props.rowCount) {
    const total = props.total ?? details.rowEstimate;
    return (
      <aside className={s.panel} aria-label="Table overview">
        <div className={s.summary}>
          <h3 className={s.title}>{design.name}</h3>
          <p className={s.sub}>
            {total !== null && `${props.total === null ? "~" : ""}${fmt.format(total)} rows · `}
            {design.columns.length} columns
          </p>
          <p className={s.hint}>Select a row to see all of its fields here.</p>
        </div>
        <h4 className={s.heading}>Columns</h4>
        <ul className={s.fieldList}>
          {design.columns.map((c) => (
            <li key={c.name}>
              <span className={s.colName}>
                {c.primaryKey && <KeyIcon size={12} />}
                {c.name}
              </span>
              <span className={s.muted}>{props.developerMode ? c.dataType : columnTypeLabel(c, driver)}</span>
            </li>
          ))}
        </ul>
        {design.foreignKeys.length > 0 && (
          <>
            <h4 className={s.heading}>Links to</h4>
            <ul className={s.fieldList}>
              {design.foreignKeys.map((f) => (
                <li key={f.name}>
                  <span className={s.colName}>{f.columns.join(", ")}</span>
                  <span className={s.muted}>
                    {f.refTable}.{f.refColumns.join(", ")}
                  </span>
                </li>
              ))}
            </ul>
          </>
        )}
        {!details.isView && (
          <button className={s.textButton} onClick={props.onEditStructure}>
            Edit columns <ArrowIcon size={13} />
          </button>
        )}
      </aside>
    );
  }

  const linkOf = (name: string) => design.foreignKeys.find((f) => f.columns.length === 1 && f.columns[0] === name);
  const viewable = (dataType: string | undefined, kind: string) => {
    const category = dataType ? categorise(dataType, driver) : null;
    return category === "json" || kind === "json" ? "json" : category === "binary" || kind === "binary" ? "binary" : null;
  };

  return (
    <aside className={s.panel} aria-label="Row details">
      <div className={s.recordHeader}>
        <span className={s.title}>Row {fmt.format(row + 1)}</span>
        <span className={s.nav}>
          <button aria-label="Previous row" disabled={row === 0} onClick={() => props.onMove(row - 1)}>
            <ChevronLeftIcon size={15} />
          </button>
          <button aria-label="Next row" disabled={row >= props.rowCount - 1} onClick={() => props.onMove(row + 1)}>
            <ChevronIcon size={15} />
          </button>
        </span>
      </div>
      <div className={s.fields}>
        {columns.map((meta, col) => {
          const d = design.columns.find((c) => c.name === meta.name);
          const value = props.value(col, row);
          const link = linkOf(meta.name);
          const writable = props.writable(col);
          return (
            <div key={`${row}:${meta.name}`} className={s.field} data-edited={props.edited(col, row) || undefined}>
              <div className={s.label}>
                <span className={s.name}>
                  {d?.primaryKey && <KeyIcon size={12} />}
                  {link && <LinkIcon size={12} />}
                  {meta.name}
                </span>
                <span className={s.type}>{d ? (props.developerMode ? d.dataType : columnTypeLabel(d, driver)) : meta.typeName}</span>
                {writable && d?.nullable && value !== null && (
                  <button className={s.nullButton} onClick={() => props.onChange(col, row, null)} title="Clear this value (NULL)">
                    Clear
                  </button>
                )}
              </div>
              {d ? (
                <FieldInput column={d} driver={driver} value={value} readOnly={!writable} onCommit={(v) => props.onChange(col, row, v ?? null)} />
              ) : (
                <div className={s.readonly}>{value ?? "NULL"}</div>
              )}
              {value !== null && viewable(d?.dataType, meta.kind) && (
                <button className={s.textButton} onClick={() => setViewed({ column: meta.name, kind: viewable(d?.dataType, meta.kind)!, value })}>
                  {viewable(d?.dataType, meta.kind) === "json" ? "View as a tree" : "View contents"} <ArrowIcon size={13} />
                </button>
              )}
              {link && value !== null && (
                <button className={s.textButton} onClick={() => props.onFollow(link, value)}>
                  Open in {link.refTable} <ArrowIcon size={13} />
                </button>
              )}
            </div>
          );
        })}
      </div>
      <ValueViewer viewed={viewed} onClose={() => setViewed(null)} />
    </aside>
  );
}
