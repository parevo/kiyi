import { useEffect, useMemo, useState } from "react";
import { errorMessage, ipc } from "../lib/ipc";
import type { Aggregate, BrowseRequest, ConnectionConfig, DatePart, GroupBy, Measure, Page, TableDetails } from "../lib/types";
import { categorise, driverFor, useCatalog } from "../state/catalog";
import { useSettings } from "../state/settings";
import type { ResultSet } from "../state/tabs";
import { ChartPanel } from "./ChartPanel";
import { BarChartIcon, PlusIcon, Spinner, TableIcon, TrashIcon } from "./icons";
import { ResultGrid } from "./ResultGrid";
import { Button, IconButton } from "./ui";
import s from "./SummaryView.module.css";

const AGGREGATES: { value: Aggregate; label: string; numeric: boolean }[] = [
  { value: "count", label: "Number of rows", numeric: false },
  { value: "countDistinct", label: "Different values of", numeric: false },
  { value: "sum", label: "Total of", numeric: true },
  { value: "avg", label: "Average of", numeric: true },
  { value: "min", label: "Lowest", numeric: false },
  { value: "max", label: "Highest", numeric: false },
];

/** "Group rows by … and show …", without SQL. Uses the table's current filters. */
export function SummaryView({
  connection,
  details,
  browse,
  onOpenSql,
}: {
  connection: ConnectionConfig;
  details: TableDetails;
  /** The data view's current filters, so the summary covers the same rows. */
  browse: BrowseRequest;
  onOpenSql(sql: string): void;
}) {
  const driver = driverFor(connection, useCatalog((st) => st.drivers));
  const developerMode = useSettings((st) => st.developerMode);
  const cols = details.design.columns;
  const category = (name: string) => categorise(cols.find((c) => c.name === name)?.dataType ?? "", driver);
  const isDate = (name: string) => ["date", "dateTime"].includes(category(name));
  const numeric = cols.filter((c) => ["number", "decimal"].includes(categorise(c.dataType, driver)) && !c.primaryKey && !c.name.endsWith("_id"));

  const [groupBy, setGroupBy] = useState<GroupBy[]>(() => {
    const choice = cols.find((c) => c.enumValues.length > 0) ?? cols.find((c) => ["boolean", "text"].includes(categorise(c.dataType, driver)) && !c.primaryKey) ?? cols.find((c) => isDate(c.name));
    return choice ? [{ column: choice.name, datePart: isDate(choice.name) ? "month" : null }] : [];
  });
  const [measures, setMeasures] = useState<Measure[]>(() => [{ aggregate: "count", column: null }, ...(numeric[0] ? [{ aggregate: "sum" as const, column: numeric[0].name }] : [])]);
  const [page, setPage] = useState<Page | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [view, setView] = useState<"chart" | "table">("chart");

  // The parent builds `browse` on every render; only a real change in it should re-run the query.
  const browseKey = JSON.stringify(browse);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const stableBrowse = useMemo(() => browse, [browseKey]);
  const request = useMemo(() => ({ browse: stableBrowse, groupBy, measures }), [stableBrowse, groupBy, measures]);
  useEffect(() => {
    let current = true;
    setLoading(true);
    setError(null);
    const t = setTimeout(() => {
      ipc.summarize(connection.id, request).then(
        (p) => current && setPage(p),
        (e) => current && setError(errorMessage(e)),
      ).finally(() => current && setLoading(false));
    }, 150);
    return () => {
      current = false;
      clearTimeout(t);
    };
  }, [connection.id, request]);

  const set: ResultSet | null = page ? { columns: page.columns, rows: page.rows, rowsAffected: null } : null;
  const filtered = browse.filters.length > 0 || !!browse.search || !!browse.rawWhere;

  return (
    <div className={s.view}>
      <div className={s.builder}>
        <div className={s.line}>
          <span className={s.word}>Group by</span>
          {groupBy.map((g, i) => (
            <span key={i} className={s.chip}>
              <select value={g.column} onChange={(e) => setGroupBy(groupBy.map((x, j) => (j === i ? { column: e.target.value, datePart: isDate(e.target.value) ? "month" : null } : x)))}>
                {cols.map((c) => (
                  <option key={c.name}>{c.name}</option>
                ))}
              </select>
              {isDate(g.column) && (
                <select value={g.datePart ?? ""} onChange={(e) => setGroupBy(groupBy.map((x, j) => (j === i ? { ...x, datePart: (e.target.value || null) as DatePart | null } : x)))} aria-label="Date grouping">
                  <option value="day">by day</option>
                  <option value="month">by month</option>
                  <option value="year">by year</option>
                  <option value="">exact value</option>
                </select>
              )}
              <IconButton label="Remove grouping" onPress={() => setGroupBy(groupBy.filter((_, j) => j !== i))}>
                <TrashIcon size={12} />
              </IconButton>
            </span>
          ))}
          {groupBy.length < 2 && (
            <Button variant="ghost" onPress={() => setGroupBy([...groupBy, { column: cols.find((c) => !groupBy.some((g) => g.column === c.name))?.name ?? cols[0].name, datePart: null }])}>
              <PlusIcon size={13} /> {groupBy.length ? "Then by" : "Add grouping"}
            </Button>
          )}
        </div>
        <div className={s.line}>
          <span className={s.word}>Show</span>
          {measures.map((m, i) => {
            const agg = AGGREGATES.find((a) => a.value === m.aggregate)!;
            const choices = agg.numeric ? numeric : cols;
            return (
              <span key={i} className={s.chip}>
                <select
                  value={m.aggregate}
                  onChange={(e) => {
                    const next = AGGREGATES.find((a) => a.value === e.target.value)!;
                    const column = next.value === "count" ? null : next.numeric ? (numeric.some((c) => c.name === m.column) ? m.column : (numeric[0]?.name ?? null)) : (m.column ?? cols[0].name);
                    setMeasures(measures.map((x, j) => (j === i ? { aggregate: next.value, column } : x)));
                  }}
                >
                  {AGGREGATES.filter((a) => !a.numeric || numeric.length).map((a) => (
                    <option key={a.value} value={a.value}>
                      {a.label}
                    </option>
                  ))}
                </select>
                {m.aggregate !== "count" && (
                  <select value={m.column ?? ""} onChange={(e) => setMeasures(measures.map((x, j) => (j === i ? { ...x, column: e.target.value } : x)))}>
                    {choices.map((c) => (
                      <option key={c.name}>{c.name}</option>
                    ))}
                  </select>
                )}
                {measures.length > 1 && (
                  <IconButton label="Remove" onPress={() => setMeasures(measures.filter((_, j) => j !== i))}>
                    <TrashIcon size={12} />
                  </IconButton>
                )}
              </span>
            );
          })}
          {measures.length < 4 && (
            <Button variant="ghost" onPress={() => setMeasures([...measures, numeric[0] ? { aggregate: "avg", column: numeric[0].name } : { aggregate: "countDistinct", column: cols[0].name }])}>
              <PlusIcon size={13} /> Add
            </Button>
          )}
          <span className={s.spacer} />
          {loading && <Spinner size={13} />}
          <div className={s.toggle} role="radiogroup" aria-label="Show as">
            <button role="radio" aria-checked={view === "chart"} onClick={() => setView("chart")}>
              <BarChartIcon size={13} /> Chart
            </button>
            <button role="radio" aria-checked={view === "table"} onClick={() => setView("table")}>
              <TableIcon size={13} /> Table
            </button>
          </div>
        </div>
        <p className={s.hint}>
          {filtered ? "Covers the rows that match the filters in the Data view." : "Covers every row of the table."} Shows up to 1,000 groups.
          {developerMode && page && (
            <button className={s.link} onClick={() => onOpenSql(page.sql)}>
              Open SQL
            </button>
          )}
        </p>
      </div>
      <div className={s.result}>
        {error && <p className={s.error} role="alert">{error}</p>}
        {!error && set && (view === "chart" ? (
          <ChartPanel columns={set.columns} rows={set.rows} />
        ) : (
          <ResultGrid set={set} rowCount={set.rows.length} kind={connection.kind} />
        ))}
      </div>
    </div>
  );
}
