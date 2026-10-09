import { Fragment, useState } from "react";
import { errorMessage, ipc } from "../lib/ipc";
import type { Comparison, DiffStatus } from "../lib/types";
import { useActiveConnection, useConnections } from "../state/connections";
import { useTabs } from "../state/tabs";
import { useUi } from "../state/ui";
import { AlertIcon, CloseIcon, CompareIcon, Spinner } from "./icons";
import { Button, IconButton } from "./ui";
import s from "./CompareView.module.css";

const LABEL: Record<DiffStatus, string> = { same: "Same", different: "Different", onlyLeft: "Missing on the right", onlyRight: "Only on the right" };
const fmt = new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 1 });

/** The open database next to another saved one: what differs, and SQL to bring the other in line. */
export function CompareView() {
  const connection = useActiveConnection();
  const connections = useConnections((st) => st.connections);
  const close = () => useUi.getState().openTool(null);
  const others = connections.filter((c) => c.id !== connection?.id);
  const [rightId, setRightId] = useState<string>(others[0]?.id ?? "");
  const [result, setResult] = useState<Comparison | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showSame, setShowSame] = useState(false);

  if (!connection) return null;
  const right = connections.find((c) => c.id === rightId);

  const run = async () => {
    if (!right) return;
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      // Connect the other database in the background, without switching to it.
      if (useConnections.getState().live[right.id]?.status !== "connected") await ipc.connect(right.id);
      setResult(await ipc.compare(connection.id, right.id));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const counts = result ? (["different", "onlyLeft", "onlyRight", "same"] as DiffStatus[]).map((st) => [st, result.tables.filter((t) => t.status === st).length] as const) : [];
  const shown = result?.tables.filter((t) => showSame || t.status !== "same") ?? [];
  const sql = result?.migration?.length ? result.migration.map((x) => `${x};`).join("\n") : "";

  return (
    <div className={s.view}>
      <div className={s.toolbar}>
        <b>
          <CompareIcon size={15} /> Compare databases
        </b>
        <span className={s.side}>{connection.name}</span>
        <span className={s.with}>with</span>
        <select value={rightId} onChange={(e) => setRightId(e.target.value)} aria-label="Compare with">
          {others.length === 0 && <option value="">Add another connection first</option>}
          {others.map((c) => (
            <option key={c.id} value={c.id}>
              {c.name} ({c.env})
            </option>
          ))}
        </select>
        <Button variant="primary" onPress={run} isDisabled={!right || busy}>
          {busy && <Spinner size={13} />} Compare
        </Button>
        <span className={s.spacer} />
        <IconButton label="Close" onPress={close}>
          <CloseIcon />
        </IconButton>
      </div>
      <div className={s.body}>
        {!result && !error && !busy && <p className={s.hint}>Compares the structure of {connection.name} (left) with another database (right): tables, columns, types and rough row counts. Nothing is changed.</p>}
        {error && (
          <p className={s.error} role="alert">
            <AlertIcon size={14} /> {error}
          </p>
        )}
        {result && (
          <>
            <div className={s.summary}>
              {counts.map(([st, n]) => (
                <span key={st} className={s.badge} data-status={st}>
                  {n} {LABEL[st].toLowerCase()}
                </span>
              ))}
              <label className={s.toggle}>
                <input type="checkbox" checked={showSame} onChange={(e) => setShowSame(e.target.checked)} /> Show identical tables
              </label>
            </div>
            <table className={s.table}>
              <thead>
                <tr>
                  <th>Table</th>
                  <th>Status</th>
                  <th>{connection.name}</th>
                  <th>{right?.name}</th>
                </tr>
              </thead>
              <tbody>
                {shown.map((t) => (
                  <Fragment key={t.name}>
                    <tr data-status={t.status}>
                      <td className={s.mono}>{t.name}</td>
                      <td>
                        <span className={s.badge} data-status={t.status}>
                          {LABEL[t.status]}
                        </span>
                      </td>
                      <td className={s.num}>{t.leftRows !== null ? `~${fmt.format(t.leftRows)} rows` : t.status === "onlyRight" ? "—" : ""}</td>
                      <td className={s.num}>{t.rightRows !== null ? `~${fmt.format(t.rightRows)} rows` : t.status === "onlyLeft" ? "—" : ""}</td>
                    </tr>
                    {t.columns.map((c) => (
                      <tr key={`${t.name}.${c.name}`} className={s.colRow}>
                        <td className={s.mono}>↳ {c.name}</td>
                        <td />
                        <td className={s.mono}>{c.left ?? <i>missing</i>}</td>
                        <td className={s.mono}>{c.right ?? <i>missing</i>}</td>
                      </tr>
                    ))}
                  </Fragment>
                ))}
                {shown.length === 0 && (
                  <tr>
                    <td colSpan={4} className={s.hint}>
                      The two databases have the same tables and columns.
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
            {result.note && <p className={s.hint}>{result.note}</p>}
            {sql && right && (
              <div className={s.sql}>
                <div className={s.sqlHead}>
                  <b>SQL to make {right.name} match</b>
                  <Button
                    onPress={() => {
                      useTabs.getState().open({ connectionId: right.id, sql: `-- Make ${right.name} match ${connection.name}. Review before running.\n${sql}`, title: `Align ${right.name}` });
                      close();
                    }}
                  >
                    Open in SQL editor for {right.name}
                  </Button>
                </div>
                <pre className={`${s.code} selectable`}>{sql}</pre>
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}
