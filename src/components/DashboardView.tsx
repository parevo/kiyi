import { useCallback, useEffect, useMemo, useState } from "react";
import { buildChart, type ChartConfig } from "../lib/chartData";
import { errorMessage } from "../lib/ipc";
import { runToEnd } from "../lib/runToEnd";
import type { Cell, ColumnMeta } from "../lib/types";
import { useConnections } from "../state/connections";
import { type Dashboard, type Tile, useDashboards } from "../state/dashboards";
import { type Tab, useTabs } from "../state/tabs";
import { Chart } from "./Chart";
import { AlertIcon, CodeIcon, DashboardIcon, RefreshIcon, Spinner, TrashIcon } from "./icons";
import { PromptDialog, type PromptRequest } from "./PromptDialog";
import { Button, IconButton } from "./ui";
import s from "./DashboardView.module.css";

const REFRESH = [0, 1, 5, 15, 60];
const time = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });

/** Saved charts side by side, each running its own read-only query. */
export function DashboardView({ tab, active }: { tab: Tab; active: boolean }) {
  const board = useDashboards((st) => st.boards.find((b) => b.id === tab.dashboardId));
  const live = useConnections((st) => (tab.connectionId ? st.live[tab.connectionId] : undefined));
  const [round, setRound] = useState(0);
  const [lastRun, setLastRun] = useState<Date | null>(null);
  const [prompt, setPrompt] = useState<PromptRequest | null>(null);
  const refreshAll = useCallback(() => {
    setRound((r) => r + 1);
    setLastRun(new Date());
  }, []);

  // Connect when opened, then draw.
  useEffect(() => {
    if (tab.connectionId && live?.status !== "connected" && live?.status !== "connecting") void useConnections.getState().activate(tab.connectionId);
  }, [tab.connectionId, live?.status]);
  useEffect(() => {
    if (live?.status === "connected") refreshAll();
  }, [live?.status, refreshAll]);
  // Automatic refresh only while the dashboard is the tab in front.
  useEffect(() => {
    if (!board?.refreshMinutes || !active) return;
    const t = setInterval(refreshAll, board.refreshMinutes * 60_000);
    return () => clearInterval(t);
  }, [board?.refreshMinutes, active, refreshAll]);

  if (!board) return <p className={s.missing}>This dashboard was deleted.</p>;
  const update = (f: (d: Dashboard) => Dashboard) => useDashboards.getState().update(board.id, f);
  const updateTile = (id: string, f: (t: Tile) => Tile) => update((d) => ({ ...d, tiles: d.tiles.map((t) => (t.id === id ? f(t) : t)) }));
  const move = (id: string, by: number) =>
    update((d) => {
      const i = d.tiles.findIndex((t) => t.id === id);
      const j = i + by;
      if (i < 0 || j < 0 || j >= d.tiles.length) return d;
      const tiles = [...d.tiles];
      [tiles[i], tiles[j]] = [tiles[j], tiles[i]];
      return { ...d, tiles };
    });

  const rename = () =>
    setPrompt({
      title: "Rename dashboard",
      fields: [{ label: "Name", value: board.name }],
      action: "Rename",
      run: ([name]) => {
        if (!name?.trim()) throw new Error("Give it a name.");
        update((d) => ({ ...d, name: name.trim() }));
        useTabs.getState().patch(tab.id, { title: name.trim() });
      },
    });

  const remove = () =>
    setPrompt({
      title: `Delete ${board.name}?`,
      fields: [],
      help: "The dashboard and its tiles are removed. The queries and data aren't touched.",
      action: "Delete",
      run: () => {
        useTabs.getState().close(tab.id, true);
        useDashboards.getState().remove(board.id);
      },
    });

  return (
    <div className={s.view}>
      <div className={s.toolbar}>
        <b>
          <DashboardIcon size={15} /> {board.name}
        </b>
        <span className={s.muted}>{lastRun ? `Updated ${time.format(lastRun)}` : ""}</span>
        <span className={s.spacer} />
        <label className={s.refresh}>
          <span>Refresh</span>
          <select value={board.refreshMinutes} onChange={(e) => update((d) => ({ ...d, refreshMinutes: Number(e.target.value) }))} aria-label="Refresh automatically">
            {REFRESH.map((m) => (
              <option key={m} value={m}>
                {m === 0 ? "by hand" : m === 60 ? "every hour" : `every ${m} min`}
              </option>
            ))}
          </select>
        </label>
        <Button onPress={refreshAll} isDisabled={live?.status !== "connected"}>
          <RefreshIcon size={14} /> Refresh
        </Button>
        <Button variant="ghost" onPress={rename}>
          Rename
        </Button>
        <IconButton label="Delete dashboard" onPress={remove}>
          <TrashIcon />
        </IconButton>
      </div>

      {board.tiles.length === 0 ? (
        <p className={s.missing}>No charts yet. In the SQL editor, show a result as a chart and choose “Add to dashboard”.</p>
      ) : (
        <div className={s.grid}>
          {board.tiles.map((t, i) => (
            <TileView
              key={t.id}
              tile={t}
              connectionId={board.connectionId}
              round={live?.status === "connected" ? round : 0}
              first={i === 0}
              last={i === board.tiles.length - 1}
              onMove={(by) => move(t.id, by)}
              onWide={() => updateTile(t.id, (x) => ({ ...x, wide: !x.wide }))}
              onRename={() =>
                setPrompt({
                  title: "Rename chart",
                  fields: [{ label: "Title", value: t.title }],
                  action: "Rename",
                  run: ([title]) => updateTile(t.id, (x) => ({ ...x, title: title?.trim() || x.title })),
                })
              }
              onRemove={() => update((d) => ({ ...d, tiles: d.tiles.filter((x) => x.id !== t.id) }))}
            />
          ))}
        </div>
      )}
      <PromptDialog request={prompt} onClose={() => setPrompt(null)} />
    </div>
  );
}

function TileView({
  tile,
  connectionId,
  round,
  first,
  last,
  onMove,
  onWide,
  onRename,
  onRemove,
}: {
  tile: Tile;
  connectionId: string;
  round: number;
  first: boolean;
  last: boolean;
  onMove(by: number): void;
  onWide(): void;
  onRename(): void;
  onRemove(): void;
}) {
  const [result, setResult] = useState<{ columns: ColumnMeta[]; rows: Cell[][] } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [own, setOwn] = useState(0);

  useEffect(() => {
    if (round === 0) return;
    let live = true;
    setBusy(true);
    setError(null);
    runToEnd(connectionId, tile.sql).then(
      (r) => live && (setResult(r), setBusy(false)),
      (e) => live && (setError(errorMessage(e)), setBusy(false)),
    );
    return () => {
      live = false;
    };
  }, [round, own, connectionId, tile.sql]);

  // Columns by name, so the tile survives a query whose columns moved.
  const data = useMemo(() => {
    if (!result) return null;
    const at = (n: string | null) => (n === null ? null : result.columns.findIndex((c) => c.name === n));
    const x = at(tile.chart.x);
    const config: ChartConfig = { kind: tile.chart.kind, x: x === -1 ? (tile.chart.kind === "number" ? null : 0) : x, y: tile.chart.y.map(at).filter((i): i is number => i !== null && i >= 0) };
    return buildChart(result.columns, result.rows, config);
  }, [result, tile.chart]);

  return (
    <section className={s.tile} data-wide={tile.wide || undefined}>
      <div className={s.tileHead}>
        <button className={s.title} onClick={onRename} title="Rename">
          {tile.title}
        </button>
        <span className={s.tileActions}>
          {busy && <Spinner size={12} />}
          <IconButton label="Refresh" onPress={() => setOwn((n) => n + 1)}>
            <RefreshIcon size={13} />
          </IconButton>
          <IconButton label="Open in SQL editor" onPress={() => useTabs.getState().open({ connectionId, sql: tile.sql, title: tile.title })}>
            <CodeIcon size={13} />
          </IconButton>
          <IconButton label={tile.wide ? "Half width" : "Full width"} onPress={onWide}>
            <span className={s.glyph}>{tile.wide ? "▭" : "▬"}</span>
          </IconButton>
          {!first && (
            <IconButton label="Move earlier" onPress={() => onMove(-1)}>
              <span className={s.glyph}>←</span>
            </IconButton>
          )}
          {!last && (
            <IconButton label="Move later" onPress={() => onMove(1)}>
              <span className={s.glyph}>→</span>
            </IconButton>
          )}
          <IconButton label="Remove from dashboard" onPress={onRemove}>
            <TrashIcon size={13} />
          </IconButton>
        </span>
      </div>
      <div className={s.body}>
        {error ? (
          <p className={s.error}>
            <AlertIcon size={14} /> {error}
          </p>
        ) : data ? (
          result && result.rows.length === 0 ? <p className={s.muted}>No rows.</p> : <Chart data={data} />
        ) : (
          <p className={s.muted}>
            <Spinner size={12} /> Loading…
          </p>
        )}
      </div>
    </section>
  );
}
