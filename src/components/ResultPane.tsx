import { useEffect, useState } from "react";
import { Button as AriaButton } from "react-aria-components";
import { configFromHint } from "../lib/chartData";
import type { ChartHint, DbKind } from "../lib/types";
import { ChartPanel } from "./ChartPanel";
import type { RunState } from "../state/tabs";
import { AlertIcon, BarChartIcon, CheckIcon, Spinner, TableIcon, WandIcon } from "./icons";
import { ResultGrid } from "./ResultGrid";
import s from "./ResultPane.module.css";
import { kbd } from "../lib/platform";

const fmt = new Intl.NumberFormat("en-US");

function duration(ms: number) {
  if (ms < 1000) return `${Math.round(ms)} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(ms < 10_000 ? 2 : 1)} s`;
  return `${Math.floor(ms / 60_000)} min ${Math.round((ms % 60_000) / 1000)} s`;
}

function useElapsed(run: RunState | null) {
  const [now, setNow] = useState(performance.now());
  useEffect(() => {
    if (run?.status !== "running") return;
    const t = setInterval(() => setNow(performance.now()), 100);
    return () => clearInterval(t);
  }, [run?.status]);
  if (!run) return 0;
  return run.elapsedMs ?? now - run.startedAt;
}

export function ResultPane({
  run,
  kind,
  hint,
  onCancel,
  onFix,
}: {
  run: RunState | null;
  kind: DbKind;
  /** The AI's idea of how to chart this result. */
  hint?: ChartHint | null;
  onCancel(): void;
  /** Offered when a run fails: let AI correct the SQL. */
  onFix?(): void;
}) {
  const elapsed = useElapsed(run);
  const [summary, setSummary] = useState("");
  const [picked, setPicked] = useState<number | null>(null);
  const [view, setView] = useState<"table" | "chart">("table");
  useEffect(() => {
    setPicked(null);
    // A question asked in plain words comes with a chart in mind; show it first.
    setView(hint ? "chart" : "table");
  }, [run?.queryId, hint]);

  const sets = run?.sets ?? [];
  const withRows = sets.map((set, i) => ({ set, i })).filter(({ set }) => set.columns.length > 0);
  // Default to the last result that has columns; a trailing UPDATE shouldn't hide a SELECT.
  const shownIndex = picked ?? withRows[withRows.length - 1]?.i ?? sets.length - 1;
  const shown = sets[shownIndex];
  const shownRows = shown ? shown.rows.length : 0;

  const chartable = !run?.error && run?.status !== "running" && !!shown && shown.columns.length > 0 && shownRows > 0;
  return (
    <div className={s.pane}>
      {(withRows.length > 1 || chartable) && (
        <div className={s.sets} role="tablist">
          {withRows.length > 1 &&
            withRows.map(({ set, i }, n) => (
              <AriaButton key={i} className={s.setTab} data-selected={i === shownIndex || undefined} onPress={() => setPicked(i)}>
                Result {n + 1} · {fmt.format(set.rows.length)}
              </AriaButton>
            ))}
          {chartable && (
            <div className={s.viewToggle} role="radiogroup" aria-label="Show results as">
              <button role="radio" aria-checked={view === "table"} onClick={() => setView("table")}>
                <TableIcon size={13} /> Table
              </button>
              <button role="radio" aria-checked={view === "chart"} onClick={() => setView("chart")}>
                <BarChartIcon size={13} /> Chart
              </button>
            </div>
          )}
        </div>
      )}

      <div className={s.content}>
        {!run && (
          <div className={`${s.center} ${s.hint}`}>
            <span>
              <kbd>{kbd("↵")}</kbd> runs the statement under the cursor
            </span>
            <span>
              <kbd>⇧</kbd> <kbd>{kbd("↵")}</kbd> runs everything
            </span>
          </div>
        )}

        {run?.error && (
          <div className={s.error} role="alert">
            <div className={s.errorTitle}>
              <AlertIcon />
              Query failed
              {run.error.code && <span className={s.errorCode}>{run.error.code}</span>}
            </div>
            <div className={`${s.errorBody} selectable`}>{run.error.message}</div>
            {onFix && (
              <button className={s.fix} onClick={onFix}>
                <WandIcon size={13} /> Fix with AI
              </button>
            )}
          </div>
        )}

        {!run?.error && shown && shown.columns.length > 0 && (view === "chart" && chartable ? (
          <ChartPanel key={`${run!.queryId}:${shownIndex}`} columns={shown.columns} rows={shown.rows} initial={configFromHint(shown.columns, hint)} />
        ) : (
          <ResultGrid key={`${run!.queryId}:${shownIndex}`} set={shown} rowCount={shownRows} kind={kind} onSummary={setSummary} />
        ))}

        {!run?.error && run?.status === "done" && shown && shown.columns.length === 0 && (
          <div className={s.center}>
            <span className={s.message}>
              <CheckIcon />
              {run.statements > 1 ? `Ran ${run.statements} statements` : "Query finished"}
              {shown.rowsAffected !== null && ` · ${fmt.format(shown.rowsAffected)} rows affected`}
            </span>
          </div>
        )}

        {run?.status === "running" && sets.length === 0 && (
          <div className={s.center}>
            <Spinner size={18} />
          </div>
        )}
      </div>

      <div className={s.status}>
        {run?.status === "running" ? (
          <>
            <span className={s.running}>
              <Spinner size={12} /> Running
            </span>
            {run.rowCount > 0 && <span>{fmt.format(run.rowCount)} rows</span>}
            <span className={s.spacer} />
            <span>{duration(elapsed)}</span>
            <AriaButton className={s.cancel} onPress={onCancel}>
              Cancel <span className={s.faint}>{kbd(".")}</span>
            </AriaButton>
          </>
        ) : run ? (
          <>
            {run.cancelled ? (
              <span>Cancelled</span>
            ) : shown && shown.columns.length > 0 ? (
              <span>{fmt.format(shownRows)} rows</span>
            ) : (
              <span>{run.error ? "Error" : "Done"}</span>
            )}
            {run.truncated && (
              <span className={s.truncated} title="Kiyi keeps at most 100,000 rows in the editor so the app stays responsive.">
                First {fmt.format(shownRows)} rows only. Add a LIMIT, or export to get everything.
              </span>
            )}
            {run.statements > 1 && <span className={s.faint}>{run.statements} statements</span>}
            {summary && <span className="selectable">{summary}</span>}
            <span className={s.spacer} />
            <span className={s.faint}>{duration(elapsed)}</span>
          </>
        ) : (
          <span className={s.faint}>Ready</span>
        )}
      </div>
    </div>
  );
}
