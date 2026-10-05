import { useEffect, useState } from "react";
import { errorMessage, ipc } from "../lib/ipc";
import type { Cell, ColumnDesign, ConnectionConfig, TableDetails } from "../lib/types";
import { categorise, columnTypeLabel, driverFor, useCatalog } from "../state/catalog";
import { toast } from "../state/toasts";
import { FieldInput } from "./FieldInput";
import { Sheet } from "./Sheet";
import { Button } from "./ui";
import f from "./Form.module.css";

/** What happens when a field is left empty, in plain words. */
function emptyMeaning(c: ColumnDesign): string | null {
  if (c.autoIncrement) return "Automatic";
  if (c.generated) return "Calculated";
  if (c.default) {
    if (/now\(\)|current_timestamp/i.test(c.default)) return "Defaults to the current time";
    if (/current_date/i.test(c.default)) return "Defaults to today";
    if (/uuid/i.test(c.default)) return "Defaults to a new UUID";
    if (/nextval/i.test(c.default)) return "Automatic";
    return `Defaults to ${c.default.replace(/^'(.*)'(::.*)?$/, "$1")}`;
  }
  return c.nullable ? "Empty (NULL)" : null;
}

export function InsertRowSheet({
  isOpen,
  connection,
  details,
  prefill,
  onClose,
  onInserted,
}: {
  isOpen: boolean;
  connection: ConnectionConfig;
  details: TableDetails;
  prefill?: Record<string, Cell>;
  onClose(): void;
  onInserted(): void;
}) {
  const driver = driverFor(connection, useCatalog((st) => st.drivers));
  const design = details.design;
  const [values, setValues] = useState<Record<string, Cell | undefined>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [tried, setTried] = useState(false);

  useEffect(() => {
    if (!isOpen) return;
    setValues(prefill ?? {});
    setError(null);
    setTried(false);
  }, [isOpen, prefill]);

  const fields = design.columns.filter((c) => !c.generated);
  const missing = fields.filter((c) => emptyMeaning(c) === null && (values[c.name] === undefined || values[c.name] === ""));

  const save = async (again: boolean) => {
    setTried(true);
    if (missing.length) return;
    setBusy(true);
    setError(null);
    try {
      const row = Object.entries(values).filter(([, v]) => v !== undefined) as [string, Cell][];
      const statements = await ipc.planRowChanges(connection.id, {
        schema: details.schema,
        table: design.name,
        binaryColumns: [],
        boolColumns: design.columns.filter((c) => categorise(c.dataType, driver) === "boolean").map((c) => c.name),
        changes: [{ type: "insert", values: row.map(([column, value]) => ({ column, value })) }],
      });
      await ipc.executeScript(connection.id, statements, "data");
      toast.success(`Row added to ${design.name}`);
      onInserted();
      if (again) {
        setValues({});
        setTried(false);
      } else onClose();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Sheet
      isOpen={isOpen}
      onClose={onClose}
      title={prefill ? "Duplicate row" : "Insert row"}
      subtitle={`into ${design.name}`}
      footer={
        <>
          <span />
          <Button onPress={onClose}>Cancel</Button>
          <Button onPress={() => save(true)} isDisabled={busy}>
            Save & add another
          </Button>
          <Button variant="primary" onPress={() => save(false)} isDisabled={busy}>
            Save row
          </Button>
        </>
      }
    >
      <form
        className={f.stack}
        onSubmit={(e) => {
          e.preventDefault();
          save(false);
        }}
      >
        {error && <div className={f.error} role="alert">{error}</div>}
        {fields.map((c, i) => {
          const meaning = emptyMeaning(c);
          const link = design.foreignKeys.find((fk) => fk.columns.length === 1 && fk.columns[0] === c.name);
          const invalid = tried && missing.includes(c);
          return (
            <div key={c.name} className={f.field}>
              <label className={f.label}>
                {c.name}
                {meaning === null && <span className={f.required}>*</span>}
                <span className={f.meta}>{columnTypeLabel(c, driver)}</span>
              </label>
              <FieldInput
                column={c}
                driver={driver}
                value={values[c.name]}
                placeholder={meaning ?? "Required"}
                autoFocus={i === Math.max(0, fields.findIndex((x) => emptyMeaning(x) === null))}
                onCommit={(v) => setValues((cur) => ({ ...cur, [c.name]: v === "" && meaning !== null ? undefined : v }))}
              />
              {invalid && <span className={f.error}>Required</span>}
              {link && !invalid && <span className={f.help}>The ID of a row in {link.refTable}.</span>}
              {c.comment && <span className={f.help}>{c.comment}</span>}
            </div>
          );
        })}
      </form>
    </Sheet>
  );
}
