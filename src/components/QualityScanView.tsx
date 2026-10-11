import { useEffect, useState } from "react";
import { errorMessage, ipc } from "../lib/ipc";
import type { QualityReport } from "../lib/types";
import { useActiveConnection } from "../state/connections";
import { useTabs } from "../state/tabs";
import { useUi } from "../state/ui";
import { AlertIcon, CheckIcon, CloseIcon, HealthIcon, RefreshIcon, Spinner } from "./icons";
import { Button, IconButton } from "./ui";
import s from "./QualityScanView.module.css";

const fmt = new Intl.NumberFormat("en-US");

/** Every table of the open database checked for messy data, worst first. */
export function QualityScanView() {
  const connection = useActiveConnection();
  const close = () => useUi.getState().openTool(null);
  const [reports, setReports] = useState<QualityReport[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const scan = async () => {
    if (!connection) return;
    setBusy(true);
    setError(null);
    try {
      setReports(await ipc.qualityScan(connection.id));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };
  useEffect(() => {
    void scan();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connection?.id]);

  if (!connection) return null;
  const warnings = (r: QualityReport) => r.findings.filter((f) => f.severity === "warning");
  const sorted = [...(reports ?? [])].sort((a, b) => warnings(b).length - warnings(a).length || a.table.localeCompare(b.table));
  const messy = sorted.filter((r) => warnings(r).length > 0);
  const open = (r: QualityReport) => {
    useTabs.getState().openTable(connection.id, r.schema, r.table, "quality");
    close();
  };

  return (
    <div className={s.view}>
      <div className={s.toolbar}>
        <b>
          <HealthIcon size={15} /> Data quality · {connection.name}
        </b>
        <span className={s.spacer} />
        <Button onPress={scan} isDisabled={busy}>
          {busy ? <Spinner size={13} /> : <RefreshIcon size={14} />} Check again
        </Button>
        <IconButton label="Close" onPress={close}>
          <CloseIcon />
        </IconButton>
      </div>
      <div className={s.body}>
        <p className={s.muted}>Each table's first 10,000 rows are checked for duplicates, inconsistent spellings, stray spaces, blank text, invalid e-mail addresses, odd numbers and dates. Open a table for the details and fixes.</p>
        {busy && !reports && (
          <p className={s.muted}>
            <Spinner size={13} /> Checking every table…
          </p>
        )}
        {error && (
          <p className={s.error} role="alert">
            <AlertIcon size={14} /> {error}
          </p>
        )}
        {reports && messy.length === 0 && (
          <div className={s.clean}>
            <CheckIcon size={18} /> Nothing looks wrong in {reports.length === 1 ? "the table" : `any of the ${reports.length} tables`}.
          </div>
        )}
        <div className={s.list}>
          {sorted.map((r) => {
            const w = warnings(r);
            return (
              <button key={`${r.schema}.${r.table}`} className={s.row} data-clean={w.length === 0 || undefined} onClick={() => open(r)}>
                <span className={s.name}>{r.table}</span>
                <span className={s.rows}>{fmt.format(r.rows)} rows</span>
                <span className={s.titles}>{w.length ? w.slice(0, 3).map((f) => f.title).join(" · ") + (w.length > 3 ? ` · ${w.length - 3} more` : "") : "Looks clean"}</span>
                {w.length > 0 ? <span className={s.badge}>{w.length}</span> : <CheckIcon size={14} className={s.ok} />}
              </button>
            );
          })}
        </div>
      </div>
    </div>
  );
}
