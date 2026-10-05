import { useEffect, useState } from "react";
import type { Cell, ColumnDesign, DriverInfo } from "../lib/types";
import { categorise } from "../state/catalog";
import f from "./Form.module.css";
import s from "./FieldInput.module.css";

function prettyJson(v: string) {
  try {
    return JSON.stringify(JSON.parse(v), null, 2);
  } catch {
    return v;
  }
}

/**
 * The right control for a column's type. Commits on blur / Enter, not per keystroke, so
 * each change is one edit (and one save when editing live rows).
 *
 * `value`: `undefined` = not set (use the column default), `null` = NULL.
 */
export function FieldInput({
  column,
  driver,
  value,
  onCommit,
  readOnly,
  placeholder,
  autoFocus,
}: {
  column: ColumnDesign;
  driver: DriverInfo | undefined;
  value: Cell | undefined;
  onCommit(value: Cell | undefined): void;
  readOnly?: boolean;
  placeholder?: string;
  autoFocus?: boolean;
}) {
  const category = categorise(column.dataType, driver);
  const isJson = category === "json";
  const shown = value === null || value === undefined ? "" : isJson ? prettyJson(value) : value;
  const [draft, setDraft] = useState(shown);
  const [invalid, setInvalid] = useState(false);
  useEffect(() => {
    setDraft(shown);
    setInvalid(false);
  }, [shown]);

  const emptyHint = value === null ? "NULL" : (placeholder ?? "");

  const commit = () => {
    if (draft === shown) return;
    if (isJson && draft.trim()) {
      try {
        onCommit(JSON.stringify(JSON.parse(draft)));
        setInvalid(false);
      } catch {
        setInvalid(true);
      }
      return;
    }
    onCommit(draft);
  };

  if (column.enumValues.length > 0) {
    return (
      <select
        className={f.control}
        value={value ?? ""}
        disabled={readOnly}
        autoFocus={autoFocus}
        onChange={(e) => onCommit(e.target.value === "" ? (value === undefined ? undefined : null) : e.target.value)}
      >
        <option value="">{value === undefined ? (placeholder ?? "Choose…") : "NULL"}</option>
        {column.enumValues.map((v) => (
          <option key={v} value={v}>
            {v}
          </option>
        ))}
      </select>
    );
  }

  // MySQL's tinyint(1) is a number in disguise; only real booleans get the switch.
  if (category === "boolean" && driver?.kind !== "mysql") {
    return (
      <div className={s.segmented} role="radiogroup" aria-label={column.name}>
        {(["true", "false"] as const).map((v) => (
          <button key={v} type="button" role="radio" aria-checked={value === v} disabled={readOnly} onClick={() => onCommit(v)}>
            {v === "true" ? "True" : "False"}
          </button>
        ))}
      </div>
    );
  }

  const long = isJson || (category === "text" && (shown.length > 60 || shown.includes("\n") || /text$/i.test(column.dataType)));
  if (long) {
    return (
      <>
        <textarea
          className={`${f.control} ${isJson ? f.mono : ""}`}
          data-invalid={invalid || undefined}
          rows={Math.min(12, Math.max(isJson ? 4 : 2, draft.split("\n").length))}
          value={draft}
          placeholder={emptyHint}
          readOnly={readOnly}
          autoFocus={autoFocus}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          spellCheck={!isJson}
        />
        {invalid && <span className={f.error}>This isn't valid JSON yet.</span>}
      </>
    );
  }

  const numeric = category === "number" || category === "decimal";
  return (
    <input
      className={`${f.control} ${numeric || category === "identifier" ? f.mono : ""}`}
      value={draft}
      inputMode={numeric ? "decimal" : undefined}
      placeholder={emptyHint || (category === "date" ? "YYYY-MM-DD" : category === "dateTime" ? "YYYY-MM-DD HH:MM:SS" : "")}
      readOnly={readOnly || category === "binary"}
      autoFocus={autoFocus}
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
