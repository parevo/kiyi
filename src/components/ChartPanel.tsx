import { useEffect, useMemo, useState } from "react";
import { buildChart, type ChartConfig, type ChartKind, isNumeric, MAX_SERIES, suggestChart } from "../lib/chartData";
import type { Cell, ColumnMeta } from "../lib/types";
import { Chart } from "./Chart";
import { BarChartIcon, LineChartIcon, PieChartIcon, SigmaIcon } from "./icons";
import s from "./ChartPanel.module.css";

const KINDS: { kind: ChartKind; label: string; icon: React.ReactNode }[] = [
  { kind: "bar", label: "Bars", icon: <BarChartIcon size={14} /> },
  { kind: "line", label: "Line", icon: <LineChartIcon size={14} /> },
  { kind: "pie", label: "Pie", icon: <PieChartIcon size={14} /> },
  { kind: "number", label: "Total", icon: <SigmaIcon size={14} /> },
];

/** A chart of a result with simple controls: what kind, which column across, which numbers up. */
export function ChartPanel({ columns, rows, initial }: { columns: ColumnMeta[]; rows: Cell[][]; initial?: ChartConfig | null }) {
  const suggested = useMemo(() => initial ?? suggestChart(columns, rows), [columns, rows, initial]);
  const [config, setConfig] = useState<ChartConfig | null>(suggested);
  useEffect(() => setConfig(suggested), [suggested]);

  const numeric = columns.map((c, i) => ({ c, i })).filter(({ c }) => isNumeric(c));
  if (!config || rows.length === 0) {
    return <p className={s.empty}>{rows.length === 0 ? "No rows to chart." : "Nothing to chart: this result has no categories, dates or numbers."}</p>;
  }
  const data = buildChart(columns, rows, config);
  const set = (patch: Partial<ChartConfig>) => setConfig({ ...config, ...patch });

  return (
    <div className={s.panel}>
      <div className={s.controls}>
        <div className={s.kinds} role="radiogroup" aria-label="Chart type">
          {KINDS.map((k) => (
            <button key={k.kind} role="radio" aria-checked={config.kind === k.kind} onClick={() => set({ kind: k.kind, x: k.kind === "number" ? null : (config.x ?? suggested?.x ?? 0) })}>
              {k.icon} {k.label}
            </button>
          ))}
        </div>
        {config.kind !== "number" && (
          <label className={s.pick}>
            <span>Across</span>
            <select value={config.x ?? ""} onChange={(e) => set({ x: Number(e.target.value) })}>
              {columns.map((c, i) => (
                <option key={i} value={i}>
                  {c.name}
                </option>
              ))}
            </select>
          </label>
        )}
        <div className={s.pick}>
          <span>{config.kind === "number" ? "Add up" : "Values"}</span>
          {numeric.length === 0 ? (
            <span className={s.faint}>Count of rows</span>
          ) : (
            <div className={s.measures}>
              {config.kind !== "number" && (
                <label>
                  <input type="checkbox" checked={config.y.length === 0} onChange={() => set({ y: [] })} /> Count
                </label>
              )}
              {numeric.map(({ c, i }) => (
                <label key={i}>
                  <input
                    type="checkbox"
                    checked={config.y.includes(i)}
                    // One measure for a figure or a pie; up to four series otherwise.
                    onChange={(e) =>
                      set({
                        y: config.kind === "number" || config.kind === "pie" ? (e.target.checked ? [i] : []) : e.target.checked ? [...config.y, i].slice(-MAX_SERIES) : config.y.filter((x) => x !== i),
                      })
                    }
                  />{" "}
                  {c.name}
                </label>
              ))}
            </div>
          )}
        </div>
      </div>
      <div className={s.chart}>
        <Chart data={data} />
      </div>
    </div>
  );
}
