import { useEffect, useMemo, useRef, useState } from "react";
import { type ChartData, needsSeparateCharts, niceTicks } from "../lib/chartData";
import s from "./Chart.module.css";

const seriesColor = (i: number) => `var(--series-${(i % 6) + 1})`;
const color = seriesColor;
const full = new Intl.NumberFormat("en-US", { maximumFractionDigits: 2 });
const compact = new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 1 });
const tick = (v: number) => (Math.abs(v) >= 10_000 ? compact.format(v) : full.format(v));

interface Tip {
  x: number;
  y: number;
  title: string;
  rows: { name: string; value: number; color: string; share?: number }[];
}

function useSize<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  useEffect(() => {
    if (!ref.current) return;
    const ro = new ResizeObserver(([e]) => setSize({ w: e.contentRect.width, h: e.contentRect.height }));
    ro.observe(ref.current);
    return () => ro.disconnect();
  }, []);
  return [ref, size] as const;
}

const textWidth = (t: string) => t.length * 6.6;

/** A chart for query results: bars, a line over time, a pie (≤ 6 slices) or one big number. */
export function Chart({ data }: { data: ChartData }) {
  // Never a second y-axis: measures of very different size get a chart each, keeping their colors.
  if ((data.kind === "bar" || data.kind === "line") && needsSeparateCharts(data.series)) {
    // Horizontal bars share their rows of categories, so their charts sit side by side.
    const sideBySide = data.kind === "bar" && isHorizontal(data);
    return (
      <div className={s.wrap}>
        {data.note && <p className={s.note}>{data.note}</p>}
        <p className={s.note}>These values differ too much in size to share one scale, so each has its own chart.</p>
        <div className={s.multiples} data-row={sideBySide || undefined}>
          {data.series.map((sr, j) => (
            <section key={sr.name} className={s.multiple}>
              <h3 className={s.multipleTitle}>
                <span className={s.swatch} style={{ background: color(j) }} />
                {sr.name}
              </h3>
              <SingleChart data={{ ...data, note: null, series: [sr] }} offset={j} />
            </section>
          ))}
        </div>
      </div>
    );
  }
  return <SingleChart data={data} offset={0} withChrome />;
}

function SingleChart({ data, offset, withChrome = false }: { data: ChartData; offset: number; withChrome?: boolean }) {
  const [ref, { w, h }] = useSize<HTMLDivElement>();
  const [tip, setTip] = useState<Tip | null>(null);
  const multi = data.series.length > 1;

  return (
    <div className={s.wrap} style={{ padding: withChrome ? undefined : 0, ["--offset" as string]: offset }}>
      {withChrome && data.note && <p className={s.note}>{data.note}</p>}
      {withChrome && (multi || data.kind === "pie") && (
        <ul className={s.legend} aria-label="Legend">
          {(data.kind === "pie" ? data.categories : data.series.map((x) => x.name)).map((name, i) => (
            <li key={name}>
              <span className={s.swatch} style={{ background: color(i) }} />
              {name}
            </li>
          ))}
        </ul>
      )}
      <div className={s.plot} ref={ref} onMouseLeave={() => setTip(null)}>
        {w > 0 && h > 0 && (
          <>
            {data.kind === "number" && <Figure data={data} />}
            {data.kind === "bar" && <Bars data={data} w={w} h={h} onTip={setTip} offset={offset} />}
            {data.kind === "line" && <Lines data={data} w={w} h={h} onTip={setTip} offset={offset} />}
            {data.kind === "pie" && <Pie data={data} w={w} h={h} onTip={setTip} />}
          </>
        )}
        {tip && (
          <div className={s.tip} style={{ left: Math.min(tip.x + 12, w - 200), top: Math.max(4, tip.y - 12) }} role="status">
            <div className={s.tipTitle}>{tip.title}</div>
            {tip.rows.map((r) => (
              <div key={r.name} className={s.tipRow}>
                <span className={s.swatch} style={{ background: r.color }} />
                <span className={s.tipName}>{r.name}</span>
                <span className={s.tipValue}>
                  {full.format(r.value)}
                  {r.share !== undefined && ` · ${(r.share * 100).toFixed(1)}%`}
                </span>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function Figure({ data }: { data: ChartData }) {
  return (
    <div className={s.figure}>
      <div className={s.figureValue}>{full.format(data.series[0].values[0])}</div>
      <div className={s.figureLabel}>{data.series[0].name}</div>
    </div>
  );
}

/** Many or long category names read better as horizontal bars; ordered axes (time) stay vertical. */
function isHorizontal(data: ChartData) {
  const longest = Math.max(...data.categories.map((c) => c.length));
  return !data.ordered && (data.categories.length > 10 || longest > 12);
}

function Bars({ data, w, h, onTip, offset }: { data: ChartData; w: number; h: number; onTip(t: Tip | null): void; offset: number }) {
  const color = (j: number) => seriesColor(j + offset);
  const n = data.categories.length;
  const longest = Math.max(...data.categories.map((c) => c.length));
  const horizontal = isHorizontal(data);
  const all = data.series.flatMap((x) => x.values);
  const ticks = niceTicks(Math.min(0, ...all), Math.max(0, ...all));
  const lo = ticks[0];
  const hi = ticks[ticks.length - 1];
  const labelW = horizontal ? Math.min(180, Math.max(...data.categories.map(textWidth)) + 12) : Math.max(...ticks.map((t) => textWidth(tick(t)))) + 14;
  const m = { top: 8, right: 16, bottom: horizontal ? 24 : 34, left: labelW };
  const pw = Math.max(10, w - m.left - m.right);
  const ph = Math.max(10, h - m.top - m.bottom);
  const band = (horizontal ? ph : pw) / n;
  const k = data.series.length;
  // Bars never fill their slot: at most 24px thick, 2px apart within a group.
  const thick = Math.max(2, Math.min(24, (band * 0.7 - (k - 1) * 2) / k));
  const group = thick * k + (k - 1) * 2;
  const scale = (v: number) => ((v - lo) / (hi - lo)) * (horizontal ? pw : ph);
  const zero = scale(0);
  // Labels never overlap: skip some when bands are narrower than a line of text.
  const everyNth = horizontal ? Math.max(1, Math.ceil(14 / band)) : Math.max(1, Math.ceil((n * Math.min(90, longest * 6.6 + 8)) / pw));
  const tipFor = (i: number, x: number, y: number) =>
    onTip({ x, y, title: data.categories[i], rows: data.series.map((sr, j) => ({ name: sr.name, value: sr.values[i], color: color(j) })) });

  return (
    <svg width={w} height={h} className={s.svg} role="img" aria-label={`Bar chart of ${data.series.map((x) => x.name).join(", ")} by category`}>
      <g transform={`translate(${m.left},${m.top})`}>
        {ticks.map((t) => {
          const p = scale(t);
          return horizontal ? (
            <g key={t}>
              <line x1={p} x2={p} y1={0} y2={ph} className={t === 0 ? s.baseline : s.grid} />
              <text x={p} y={ph + 16} className={s.tick} textAnchor="middle">
                {tick(t)}
              </text>
            </g>
          ) : (
            <g key={t}>
              <line x1={0} x2={pw} y1={ph - p} y2={ph - p} className={t === 0 ? s.baseline : s.grid} />
              <text x={-8} y={ph - p + 4} className={s.tick} textAnchor="end">
                {tick(t)}
              </text>
            </g>
          );
        })}
        {data.categories.map((c, i) => {
          const start = i * band + (band - group) / 2;
          return (
            <g key={c} onMouseMove={(e) => tipFor(i, e.nativeEvent.offsetX, e.nativeEvent.offsetY)}>
              {/* A full-band hit target, bigger than the bars. */}
              <rect x={horizontal ? 0 : i * band} y={horizontal ? i * band : 0} width={horizontal ? pw : band} height={horizontal ? band : ph} fill="transparent" />
              {data.series.map((sr, j) => {
                const v = sr.values[i];
                const len = Math.abs(scale(v) - zero);
                const off = start + j * (thick + 2);
                const r = Math.min(4, thick / 2, len / 2);
                // Rounded at the data end, square at the baseline.
                const path = horizontal
                  ? v >= 0
                    ? roundedEnd(zero, off, len, thick, r, "right")
                    : roundedEnd(zero - len, off, len, thick, r, "left")
                  : v >= 0
                    ? roundedEnd(off, ph - zero - len, thick, len, r, "top")
                    : roundedEnd(off, ph - zero, thick, len, r, "bottom");
                return <path key={sr.name} d={path} fill={color(j)} />;
              })}
              {i % everyNth === 0 &&
                (horizontal ? (
                  <text x={-8} y={i * band + band / 2 + 4} className={s.label} textAnchor="end">
                    {clip(c, 26)}
                  </text>
                ) : (
                  <text x={i * band + band / 2} y={ph + 18} className={s.label} textAnchor="middle">
                    {clip(c, Math.max(4, Math.floor((band * everyNth) / 6.6)))}
                  </text>
                ))}
            </g>
          );
        })}
      </g>
    </svg>
  );
}

function roundedEnd(x: number, y: number, w: number, h: number, r: number, end: "top" | "bottom" | "left" | "right"): string {
  if (w <= 0 || h <= 0) return "";
  switch (end) {
    case "top":
      return `M${x},${y + h}V${y + r}Q${x},${y} ${x + r},${y}H${x + w - r}Q${x + w},${y} ${x + w},${y + r}V${y + h}Z`;
    case "bottom":
      return `M${x},${y}V${y + h - r}Q${x},${y + h} ${x + r},${y + h}H${x + w - r}Q${x + w},${y + h} ${x + w},${y + h - r}V${y}Z`;
    case "right":
      return `M${x},${y}H${x + w - r}Q${x + w},${y} ${x + w},${y + r}V${y + h - r}Q${x + w},${y + h} ${x + w - r},${y + h}H${x}Z`;
    case "left":
      return `M${x + w},${y}H${x + r}Q${x},${y} ${x},${y + r}V${y + h - r}Q${x},${y + h} ${x + r},${y + h}H${x + w}Z`;
  }
}

const clip = (t: string, n: number) => (t.length > n ? `${t.slice(0, Math.max(1, n - 1))}…` : t);

function Lines({ data, w, h, onTip, offset }: { data: ChartData; w: number; h: number; onTip(t: Tip | null): void; offset: number }) {
  const color = (j: number) => seriesColor(j + offset);
  const [hover, setHover] = useState<number | null>(null);
  const n = data.categories.length;
  const all = data.series.flatMap((x) => x.values);
  const ticks = niceTicks(Math.min(...all), Math.max(...all));
  const lo = ticks[0];
  const hi = ticks[ticks.length - 1];
  const m = { top: 10, right: 20, bottom: 34, left: Math.max(...ticks.map((t) => textWidth(tick(t)))) + 14 };
  const pw = Math.max(10, w - m.left - m.right);
  const ph = Math.max(10, h - m.top - m.bottom);
  const x = (i: number) => (n === 1 ? pw / 2 : (i / (n - 1)) * pw);
  const y = (v: number) => ph - ((v - lo) / (hi - lo)) * ph;
  const labels = Math.max(2, Math.floor(pw / 90));
  const labelEvery = Math.max(1, Math.ceil(n / labels));
  const paths = useMemo(() => data.series.map((sr) => sr.values.map((v, i) => `${i ? "L" : "M"}${x(i).toFixed(1)},${y(v).toFixed(1)}`).join("")), [data, pw, ph, lo, hi]); // eslint-disable-line react-hooks/exhaustive-deps

  const onMove = (e: React.MouseEvent<SVGRectElement>) => {
    const px = e.nativeEvent.offsetX - m.left;
    const i = Math.max(0, Math.min(n - 1, Math.round((px / pw) * (n - 1))));
    setHover(i);
    onTip({ x: m.left + x(i), y: m.top + Math.min(...data.series.map((sr) => y(sr.values[i]))), title: data.categories[i], rows: data.series.map((sr, j) => ({ name: sr.name, value: sr.values[i], color: color(j) })) });
  };

  return (
    <svg width={w} height={h} className={s.svg} role="img" aria-label={`Line chart of ${data.series.map((sr) => sr.name).join(", ")} over ${data.categories[0]} to ${data.categories[n - 1]}`}>
      <g transform={`translate(${m.left},${m.top})`}>
        {ticks.map((t) => (
          <g key={t}>
            <line x1={0} x2={pw} y1={y(t)} y2={y(t)} className={t === 0 ? s.baseline : s.grid} />
            <text x={-8} y={y(t) + 4} className={s.tick} textAnchor="end">
              {tick(t)}
            </text>
          </g>
        ))}
        {data.categories.map((c, i) =>
          i % labelEvery === 0 || i === n - 1 ? (
            <text key={i} x={x(i)} y={ph + 18} className={s.label} textAnchor={i === 0 ? "start" : i === n - 1 ? "end" : "middle"}>
              {clip(c, 16)}
            </text>
          ) : null,
        )}
        {paths.map((d, j) => (
          <path key={j} d={d} fill="none" stroke={color(j)} strokeWidth={2} strokeLinejoin="round" strokeLinecap="round" />
        ))}
        {/* End dots on each line, and dots at the hovered point, ringed in the surface color. */}
        {data.series.map((sr, j) => (
          <circle key={`end${j}`} cx={x(n - 1)} cy={y(sr.values[n - 1])} r={4} fill={color(j)} className={s.ring} />
        ))}
        {hover !== null && (
          <>
            <line x1={x(hover)} x2={x(hover)} y1={0} y2={ph} className={s.crosshair} />
            {data.series.map((sr, j) => (
              <circle key={j} cx={x(hover)} cy={y(sr.values[hover])} r={4.5} fill={color(j)} className={s.ring} />
            ))}
          </>
        )}
        <rect width={pw} height={ph} fill="transparent" onMouseMove={onMove} onMouseLeave={() => setHover(null)} />
      </g>
    </svg>
  );
}

function Pie({ data, w, h, onTip }: { data: ChartData; w: number; h: number; onTip(t: Tip | null): void }) {
  const values = data.series[0].values;
  const total = values.reduce((a, b) => a + b, 0) || 1;
  const r = Math.max(20, Math.min(w, h) / 2 - 12);
  const cx = w / 2;
  const cy = h / 2;
  let angle = -Math.PI / 2;
  return (
    <svg width={w} height={h} className={s.svg} role="img" aria-label={`Pie chart of ${data.series[0].name}`}>
      {values.map((v, i) => {
        const a0 = angle;
        const a1 = angle + (v / total) * Math.PI * 2;
        angle = a1;
        const large = a1 - a0 > Math.PI ? 1 : 0;
        const p = (a: number) => `${cx + r * Math.cos(a)},${cy + r * Math.sin(a)}`;
        const d = values.length === 1 ? `M${cx - r},${cy}a${r},${r} 0 1,0 ${2 * r},0a${r},${r} 0 1,0 ${-2 * r},0` : `M${cx},${cy}L${p(a0)}A${r},${r} 0 ${large} 1 ${p(a1)}Z`;
        return (
          <path
            key={i}
            d={d}
            fill={color(i)}
            className={s.slice}
            onMouseMove={(e) => onTip({ x: e.nativeEvent.offsetX, y: e.nativeEvent.offsetY, title: data.categories[i], rows: [{ name: data.series[0].name, value: v, color: color(i), share: v / total }] })}
          />
        );
      })}
    </svg>
  );
}
