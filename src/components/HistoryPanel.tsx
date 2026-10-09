import { useMemo, useState } from "react";
import { useHistory } from "../state/history";
import { CloseIcon, TrashIcon } from "./icons";
import { IconButton } from "./ui";
import s from "./HistoryPanel.module.css";

const when = (at: number) => {
  const d = new Date(at);
  const today = new Date();
  return d.toDateString() === today.toDateString() ? d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" }) : d.toLocaleDateString();
};

/** Saved queries and everything run on this connection, searchable. Clicking inserts into the editor. */
export function HistoryPanel({ connectionId, onPick, onClose }: { connectionId: string; onPick(sql: string): void; onClose(): void }) {
  const entries = useHistory((st) => st.entries);
  const saved = useHistory((st) => st.saved);
  const { remove, clear } = useHistory.getState();
  const [needle, setNeedle] = useState("");
  const [tab, setTab] = useState<"saved" | "history">(saved.some((q) => q.connectionId === connectionId || !q.connectionId) ? "saved" : "history");

  const match = (text: string) => !needle.trim() || text.toLowerCase().includes(needle.trim().toLowerCase());
  const mine = useMemo(() => entries.filter((e) => e.connectionId === connectionId), [entries, connectionId]);
  const savedHere = saved.filter((q) => (q.connectionId === connectionId || !q.connectionId) && (match(q.name) || match(q.sql)));
  const shown = mine.filter((e) => match(e.sql)).slice(0, 300);

  return (
    <aside className={s.panel} aria-label="Query history">
      <div className={s.head}>
        <div className={s.tabs} role="tablist">
          <button role="tab" aria-selected={tab === "saved"} onClick={() => setTab("saved")}>
            Saved
          </button>
          <button role="tab" aria-selected={tab === "history"} onClick={() => setTab("history")}>
            History
          </button>
        </div>
        <IconButton label="Close" onPress={onClose}>
          <CloseIcon />
        </IconButton>
      </div>
      <input className={s.search} value={needle} onChange={(e) => setNeedle(e.target.value)} placeholder="Search" spellCheck={false} aria-label="Search queries" />
      <div className={s.list}>
        {tab === "saved" &&
          (savedHere.length ? (
            savedHere.map((q) => (
              <div key={q.id} className={s.item}>
                <button className={s.pick} onClick={() => onPick(q.sql)} title="Insert into the editor">
                  <span className={s.name}>{q.name}</span>
                  <code className={s.sql}>{q.sql}</code>
                </button>
                <IconButton label={`Delete ${q.name}`} onPress={() => remove(q.id)}>
                  <TrashIcon size={13} />
                </IconButton>
              </div>
            ))
          ) : (
            <p className={s.empty}>No saved queries yet. Use “Save” above the editor to keep one.</p>
          ))}
        {tab === "history" && (
          <>
            {shown.map((e) => (
              <button key={e.id} className={`${s.item} ${s.pick}`} onClick={() => onPick(e.sql)} title="Insert into the editor">
                <span className={s.meta}>
                  <span data-ok={e.ok || undefined} className={s.dot} />
                  {when(e.at)}
                  {e.rows !== null && ` · ${e.rows.toLocaleString("en-US")} rows`}
                  {e.ms !== null && ` · ${e.ms < 1000 ? `${e.ms} ms` : `${(e.ms / 1000).toFixed(1)} s`}`}
                </span>
                <code className={s.sql}>{e.sql}</code>
              </button>
            ))}
            {shown.length === 0 && <p className={s.empty}>{mine.length ? "Nothing matches." : "Queries you run here will show up in this list."}</p>}
            {mine.length > 0 && (
              <button className={s.clear} onClick={() => confirm("Clear the query history for this connection?") && clear(connectionId)}>
                Clear history
              </button>
            )}
          </>
        )}
      </div>
    </aside>
  );
}
