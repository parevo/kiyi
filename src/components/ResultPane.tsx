import { useEffect, useRef, useState } from "react";
import { Button as AriaButton } from "react-aria-components";
import { type ChartConfig, configFromHint } from "../lib/chartData";
import { errorMessage, ipc } from "../lib/ipc";
import type { Cell, ChartHint, ColumnMeta, ConnectionConfig, DbKind, EditableResult, RowChange } from "../lib/types";
import { toast } from "../state/toasts";
import type { ReviewRequest } from "./ReviewDialog";
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

const rows = (n: number) => `${fmt.format(n)} ${n === 1 ? "row" : "rows"}`;

export function ResultPane({
  run,
  kind,
  hint,
  onCancel,
  onFix,
  onPin,
  connection,
  onReview,
  onDirty,
}: {
  run: RunState | null;
  kind: DbKind;
  /** Where the query ran; a result of one table's rows can then be edited and saved back. */
  connection?: ConnectionConfig;
  /** Production saves are reviewed first. */
  onReview?(request: ReviewRequest): void;
  /** Unsaved edits exist (closing the tab asks first). */
  onDirty?(dirty: boolean): void;
  /** The AI's idea of how to chart this result. */
  hint?: ChartHint | null;
  onCancel(): void;
  /** Offered when a run fails: let AI correct the SQL. */
  onFix?(): void;
  /** Keep this result's chart on a dashboard. */
  onPin?(config: ChartConfig, columns: ColumnMeta[]): void;
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

  // Editing in place: known once the run is done, and only for a single result of one table.
  const [editInfo, setEditInfo] = useState<EditableResult | null>(null);
  const changes = useRef(new Map<number, Map<number, Cell>>());
  const [version, setVersion] = useState(0);
  const [saving, setSaving] = useState(false);
  const pending = changes.current.size;
  const resetEdits = () => {
    changes.current = new Map();
    setVersion((v) => v + 1);
  };
  useEffect(() => {
    setEditInfo(null);
    resetEdits();
    if (!connection || !run || run.status !== "done" || run.error || run.cancelled || run.statements !== 1) return;
    const withColumns = run.sets.filter((x) => x.columns.length > 0);
    if (withColumns.length !== 1 || withColumns[0].rows.length === 0) return;
    let stale = false;
    ipc.editableResult(connection.id, run.sql, withColumns[0].columns.map((c) => c.name)).then(
      (r) => !stale && setEditInfo(r),
      () => {},
    );
    return () => {
      stale = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [run?.queryId, run?.status, connection?.id]);
  useEffect(() => {
    onDirty?.(pending > 0);
  }, [pending > 0]); // eslint-disable-line react-hooks/exhaustive-deps

  const sets = run?.sets ?? [];
  const withRows = sets.map((set, i) => ({ set, i })).filter(({ set }) => set.columns.length > 0);
  // Default to the last result that has columns; a trailing UPDATE shouldn't hide a SELECT.
  const shownIndex = picked ?? withRows[withRows.length - 1]?.i ?? sets.length - 1;
  const shown = sets[shownIndex];
  const shownRows = shown ? shown.rows.length : 0;

  const editing = !!editInfo && !!shown && shown.columns.length > 0;
  const edit = editing
    ? {
        // Binary values are shown as text and can't be typed back.
        writable: editInfo!.columns.map((c, i) => c !== null && shown!.columns[i]?.kind !== "binary"),
        changes: changes.current,
        version,
        onChange(row: number, col: number, value: Cell) {
          const original = shown!.rows[row]?.[col] ?? null;
          const m = new Map(changes.current);
          const cols = new Map(m.get(row) ?? []);
          if (value === original) cols.delete(col);
          else cols.set(col, value);
          if (cols.size) m.set(row, cols);
          else m.delete(row);
          changes.current = m;
          setVersion((v) => v + 1);
        },
      }
    : undefined;

  const save = async () => {
    if (!connection || !editInfo || !shown || !pending) return;
    const info = editInfo;
    const set = shown;
    const at = (name: string) => info.columns.indexOf(name);
    const list: RowChange[] = [...changes.current].map(([row, cols]) => ({
      type: "update",
      key: info.key.map((k) => ({ column: k, value: set.rows[row][at(k)] })),
      values: [...cols].map(([c, value]) => ({ column: info.columns[c]!, value })),
    }));
    const of = (k: string) => set.columns.flatMap((c, i) => (c.kind === k && info.columns[i] ? [info.columns[i]!] : []));
    const applied = new Map(changes.current);
    const finish = () => {
      for (const [row, cols] of applied) for (const [c, v] of cols) set.rows[row][c] = v;
      resetEdits();
      toast.success(list.length === 1 ? "Saved 1 row" : `Saved ${list.length} rows`);
    };
    setSaving(true);
    try {
      const statements = await ipc.planRowChanges(connection.id, { schema: info.schema, table: info.table, binaryColumns: of("binary"), boolColumns: of("bool"), changes: list });
      if (connection.env === "production" && onReview) {
        onReview({
          title: "Save changes",
          subtitle: `${info.table} · production`,
          summary: [{ text: `${list.length} ${list.length === 1 ? "row" : "rows"} will be updated` }],
          statements,
          action: "Save",
          confirmWord: info.table,
          run: async () => {
            await ipc.executeScript(connection.id, statements, "data");
            finish();
          },
        });
      } else {
        await ipc.executeScript(connection.id, statements, "data");
        finish();
      }
    } catch (e) {
      toast.error(`Not saved: ${errorMessage(e)}`);
    } finally {
      setSaving(false);
    }
  };

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
          <ChartPanel
            key={`${run!.queryId}:${shownIndex}`}
            columns={shown.columns}
            rows={shown.rows}
            initial={configFromHint(shown.columns, hint)}
            onPin={onPin && run && run.sets.filter((x) => x.columns.length > 0).length === 1 ? (config) => onPin(config, shown.columns) : undefined}
          />
        ) : (
          <ResultGrid key={`${run!.queryId}:${shownIndex}`} set={shown} rowCount={shownRows} kind={kind} onSummary={setSummary} edit={edit} />
        ))}

        {!run?.error && run?.status === "done" && shown && shown.columns.length === 0 && (
          <div className={s.center}>
            <span className={s.message}>
              <CheckIcon />
              {run.statements > 1 ? `Ran ${run.statements} statements` : "Query finished"}
              {shown.rowsAffected !== null && ` · ${rows(shown.rowsAffected)} affected`}
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
            {run.rowCount > 0 && <span>{rows(run.rowCount)}</span>}
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
              <span>{rows(shownRows)}</span>
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
            {editing && pending > 0 ? (
              <>
                <span className={s.unsaved}>{pending === 1 ? "1 unsaved row" : `${fmt.format(pending)} unsaved rows`}</span>
                <AriaButton className={s.cancel} onPress={resetEdits} isDisabled={saving}>
                  Discard
                </AriaButton>
                <AriaButton className={s.save} onPress={save} isDisabled={saving}>
                  {saving ? <Spinner size={12} /> : null} Save
                </AriaButton>
              </>
            ) : editing ? (
              <span className={s.faint} title={`Each row is one row of ${editInfo!.table}. Changes are saved by its key.`}>
                Editable · {editInfo!.table}
              </span>
            ) : null}
            <span className={s.faint}>{duration(elapsed)}</span>
          </>
        ) : (
          <span className={s.faint}>Ready</span>
        )}
      </div>
    </div>
  );
}
