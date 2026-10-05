import { useEffect, useState } from "react";
import { errorMessage, ipc } from "../lib/ipc";
import type { ConnectionConfig, CsvPreview, TableDetails } from "../lib/types";
import { toast } from "../state/toasts";
import { Sheet } from "./Sheet";
import { Button, Switch } from "./ui";
import f from "./Form.module.css";
import s from "./ImportSheet.module.css";

const norm = (v: string) => v.toLowerCase().replace(/[^a-z0-9]/g, "");

/** Maps a CSV file's columns onto a table and imports it in one transaction. */
export function ImportSheet({
  path,
  connection,
  details,
  onClose,
  onImported,
}: {
  path: string | null;
  connection: ConnectionConfig;
  details: TableDetails;
  onClose(): void;
  onImported(): void;
}) {
  const [preview, setPreview] = useState<CsvPreview | null>(null);
  const [mapping, setMapping] = useState<(string | null)[]>([]);
  const [hasHeader, setHasHeader] = useState(true);
  const [emptyAsNull, setEmptyAsNull] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const columns = details.design.columns.filter((c) => !c.generated);

  useEffect(() => {
    if (!path) return;
    setPreview(null);
    setError(null);
    ipc.csvPreview(path).then(
      (p) => {
        setPreview(p);
        // Match by name, ignoring case, spaces and underscores.
        setMapping(p.headers.map((h) => columns.find((c) => norm(c.name) === norm(h))?.name ?? null));
      },
      (e) => setError(errorMessage(e)),
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path]);

  const mapped = mapping.filter(Boolean).length;
  const rows = preview ? preview.total + (hasHeader ? 0 : 1) : 0;
  const fileName = path?.split(/[\\/]/).pop();

  const run = async () => {
    if (!path) return;
    setBusy(true);
    setError(null);
    try {
      const n = await ipc.importCsv(connection.id, path, { schema: details.schema, table: details.design.name, mapping, hasHeader, emptyAsNull });
      toast.success(`Imported ${n.toLocaleString("en-US")} rows into ${details.design.name}`);
      onImported();
      onClose();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Sheet
      isOpen={path !== null}
      onClose={onClose}
      width={620}
      title="Import from CSV"
      subtitle={fileName ? `${fileName} → ${details.design.name}` : undefined}
      footer={
        <>
          <span className={s.summary}>{preview && `${rows.toLocaleString("en-US")} rows · ${mapped} of ${preview.headers.length} columns`}</span>
          <Button onPress={onClose}>Cancel</Button>
          <Button variant="primary" onPress={run} isDisabled={busy || !preview || mapped === 0}>
            {busy ? "Importing…" : `Import ${rows.toLocaleString("en-US")} rows`}
          </Button>
        </>
      }
    >
      <div className={f.stack}>
        {error && <div className={f.error} role="alert">{error}</div>}
        {!preview && !error && <p className={f.help}>Reading the file…</p>}
        {preview && (
          <>
            <p className={f.help}>Choose where each column of the file goes. Columns set to “Skip” aren't imported; table columns you don't fill get their default value.</p>
            <div className={s.map}>
              <div className={s.mapHead}>
                <span>In the file</span>
                <span>Example</span>
                <span>Goes into</span>
              </div>
              {preview.headers.map((h, i) => (
                <div key={i} className={s.mapRow}>
                  <span className={s.csvName}>{hasHeader ? h || `Column ${i + 1}` : `Column ${i + 1}`}</span>
                  <span className={s.sample}>{(hasHeader ? preview.rows[0]?.[i] : h) || "—"}</span>
                  <select className={f.control} value={mapping[i] ?? ""} onChange={(e) => setMapping((m) => m.map((x, j) => (j === i ? e.target.value || null : x)))} aria-label={`Target for ${h}`}>
                    <option value="">Skip</option>
                    {columns.map((c) => (
                      <option key={c.name} value={c.name}>
                        {c.name}
                      </option>
                    ))}
                  </select>
                </div>
              ))}
            </div>
            <div className={f.toggles}>
              <Switch isSelected={hasHeader} onChange={setHasHeader}>
                The first row has column names
              </Switch>
              <Switch isSelected={emptyAsNull} onChange={setEmptyAsNull}>
                Treat empty values as empty (NULL)
              </Switch>
            </div>
            <p className={f.help}>Everything is imported in one transaction: if any row is rejected, nothing is added.</p>
          </>
        )}
      </div>
    </Sheet>
  );
}
