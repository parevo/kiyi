import { useCallback, useEffect, useState } from "react";
import { errorMessage, ipc } from "../lib/ipc";
import type { ConnectionConfig, Filter, QualityFinding, QualityReport, TableDetails } from "../lib/types";
import { toast } from "../state/toasts";
import { AlertIcon, CheckIcon, RefreshIcon, Spinner } from "./icons";
import type { ReviewRequest } from "./ReviewDialog";
import { Button } from "./ui";
import s from "./QualityView.module.css";

const fmt = new Intl.NumberFormat("en-US");

/** What looks wrong in a table's data, with a reviewed fix where one is safe. */
export function QualityView({
  connection,
  details,
  onShowRows,
  onReview,
  onChanged,
}: {
  connection: ConnectionConfig;
  details: TableDetails;
  onShowRows(filters: Filter[]): void;
  onReview(r: ReviewRequest): void;
  onChanged(): void;
}) {
  const [report, setReport] = useState<QualityReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const check = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      setReport(await ipc.qualityCheck(connection.id, details.schema, details.design.name));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }, [connection.id, details.schema, details.design.name]);

  useEffect(() => {
    void check();
  }, [check]);

  const fix = (f: QualityFinding) => {
    if (!f.fix) return;
    const production = connection.env === "production";
    onReview({
      title: f.fix.label,
      subtitle: `${details.design.name}${production ? " · production" : ""}`,
      summary: [{ text: `Changes ${f.column ?? "rows"} in every row of ${details.design.name} that matches, not only the ${fmt.format(report?.checked ?? 0)} checked.`, danger: production }],
      statements: f.fix.statements,
      action: "Fix",
      confirmWord: connection.database ?? connection.name,
      destructive: production ? f.fix.statements.length : 0,
      run: async () => {
        await ipc.executeScript(connection.id, f.fix!.statements, "bulk");
        toast.success("Fixed");
        onChanged();
        await check();
      },
    });
  };

  const warnings = report?.findings.filter((f) => f.severity === "warning") ?? [];
  const notes = report?.findings.filter((f) => f.severity === "info") ?? [];
  const sampled = report && report.checked < report.rows;

  return (
    <div className={s.view}>
      <div className={s.head}>
        <div>
          <h2>Data quality</h2>
          <p className={s.muted}>
            {report
              ? sampled
                ? `Checked the first ${fmt.format(report.checked)} of ${fmt.format(report.rows)} rows. Fixes apply to the whole table.`
                : `Checked all ${fmt.format(report.checked)} rows.`
              : "Looking for duplicates, inconsistent spellings, stray spaces, odd values…"}
          </p>
        </div>
        <Button onPress={check} isDisabled={busy}>
          {busy ? <Spinner size={13} /> : <RefreshIcon size={14} />} Check again
        </Button>
      </div>

      {error && (
        <p className={s.error} role="alert">
          <AlertIcon size={14} /> {error}
        </p>
      )}

      {report && report.checked === 0 && <p className={s.muted}>The table has no rows to check yet.</p>}
      {report && report.checked > 0 && warnings.length === 0 && (
        <div className={s.clean}>
          <CheckIcon size={18} />
          <span>Nothing looks wrong in the rows checked.</span>
        </div>
      )}

      <div className={s.list}>
        {[...warnings, ...notes].map((f, i) => (
          <article key={`${f.kind}:${f.column}:${i}`} className={s.card} data-severity={f.severity}>
            <div className={s.cardHead}>
              <h3>{f.title}</h3>
              {f.rows > 0 && f.severity === "warning" && <span className={s.count}>{fmt.format(f.rows)}</span>}
            </div>
            <p>{f.detail}</p>
            {f.examples.length > 0 && (
              <div className={s.examples}>
                {f.examples.map((e, j) => (
                  <span key={j} className={s.example}>
                    <code className="selectable">{e.value ?? "NULL"}</code>
                    {e.count > 1 && <span className={s.times}>×{fmt.format(e.count)}</span>}
                  </span>
                ))}
              </div>
            )}
            {(f.fix || f.show.length > 0) && (
              <div className={s.actions}>
                {f.fix && (
                  <Button variant="primary" onPress={() => fix(f)} isDisabled={connection.readOnly}>
                    {f.fix.label}
                  </Button>
                )}
                {f.show.length > 0 && (
                  <Button variant="ghost" onPress={() => onShowRows(f.show)}>
                    Show the rows
                  </Button>
                )}
                {f.fix && connection.readOnly && <span className={s.muted}>Read-only connection</span>}
              </div>
            )}
          </article>
        ))}
      </div>
    </div>
  );
}
