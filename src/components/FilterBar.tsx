import { useEffect, useState } from "react";
import type { DbKind, Filter, FilterOp } from "../lib/types";
import { CloseIcon, PlusIcon } from "./icons";
import { Button, IconButton } from "./ui";
import s from "./FilterBar.module.css";

const OPS: { op: FilterOp; label: string }[] = [
  { op: "eq", label: "=" },
  { op: "ne", label: "≠" },
  { op: "lt", label: "<" },
  { op: "gt", label: ">" },
  { op: "le", label: "≤" },
  { op: "ge", label: "≥" },
  { op: "contains", label: "içerir" },
  { op: "notContains", label: "içermez" },
  { op: "startsWith", label: "ile başlar" },
  { op: "endsWith", label: "ile biter" },
  { op: "in", label: "şunlardan biri" },
  { op: "isNull", label: "NULL" },
  { op: "notNull", label: "NULL değil" },
];

const NO_VALUE: FilterOp[] = ["isNull", "notNull"];

export interface FilterState {
  filters: Filter[];
  rawWhere: string | null;
}

export function FilterBar({
  columns,
  value,
  kind,
  onApply,
}: {
  columns: string[];
  value: FilterState;
  kind: DbKind;
  onApply(next: FilterState): void;
}) {
  const [draft, setDraft] = useState<Filter[]>(value.filters);
  const [raw, setRaw] = useState(value.rawWhere ?? "");
  const [showRaw, setShowRaw] = useState(!!value.rawWhere);

  useEffect(() => {
    setDraft(value.filters.length ? value.filters : [{ column: columns[0] ?? "", op: "contains", value: "" }]);
    setRaw(value.rawWhere ?? "");
  }, [value, columns]);

  const apply = () =>
    onApply({
      filters: draft.filter((f) => f.column && (NO_VALUE.includes(f.op) || f.value !== "")),
      rawWhere: showRaw && raw.trim() ? raw.trim() : null,
    });

  const update = (i: number, patch: Partial<Filter>) => setDraft((d) => d.map((f, j) => (j === i ? { ...f, ...patch } : f)));
  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      apply();
    }
  };

  return (
    <div className={s.bar} onKeyDown={onKey}>
      {draft.map((f, i) => (
        <div key={i} className={s.row}>
          <span className={s.join}>{i === 0 ? "WHERE" : "AND"}</span>
          <select className={s.select} value={f.column} onChange={(e) => update(i, { column: e.target.value })} aria-label="Sütun">
            {columns.map((c) => (
              <option key={c}>{c}</option>
            ))}
          </select>
          <select className={s.select} value={f.op} onChange={(e) => update(i, { op: e.target.value as FilterOp })} aria-label="Operatör">
            {OPS.map((o) => (
              <option key={o.op} value={o.op}>
                {o.label}
              </option>
            ))}
          </select>
          <input
            className={s.value}
            value={f.value}
            disabled={NO_VALUE.includes(f.op)}
            placeholder={f.op === "in" ? "a, b, c" : "değer"}
            onChange={(e) => update(i, { value: e.target.value })}
            autoFocus={i === draft.length - 1}
            spellCheck={false}
            aria-label="Değer"
          />
          <IconButton label="Koşulu kaldır" onPress={() => setDraft((d) => d.filter((_, j) => j !== i))}>
            <CloseIcon size={14} />
          </IconButton>
        </div>
      ))}
      {showRaw && (
        <div className={s.row}>
          <span className={s.join}>{draft.length ? "AND" : "WHERE"}</span>
          <input
            className={s.raw}
            value={raw}
            onChange={(e) => setRaw(e.target.value)}
            placeholder={kind === "mysql" ? "total > 100 AND note LIKE '%x%'" : "total > 100 AND note ILIKE '%x%'"}
            spellCheck={false}
            aria-label="Ham WHERE koşulu"
          />
        </div>
      )}
      <div className={s.actions}>
        <Button variant="ghost" onPress={() => setDraft((d) => [...d, { column: columns[0] ?? "", op: "eq", value: "" }])}>
          <PlusIcon size={14} /> Koşul
        </Button>
        <Button variant="ghost" onPress={() => setShowRaw((v) => !v)}>
          {showRaw ? "SQL koşulunu kaldır" : "SQL koşulu yaz"}
        </Button>
        <span className={s.spacer} />
        <span className={s.hint}>↵ uygula</span>
        <Button
          variant="ghost"
          onPress={() => {
            setDraft([]);
            setRaw("");
            onApply({ filters: [], rawWhere: null });
          }}
        >
          Temizle
        </Button>
        <Button variant="primary" onPress={apply}>
          Uygula
        </Button>
      </div>
    </div>
  );
}
