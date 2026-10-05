import { useRef, useState } from "react";
import { useConnections } from "../state/connections";
import { type Tab, useTabs } from "../state/tabs";
import { QueryEditor, type RunRequest } from "./QueryEditor";
import { ResultPane } from "./ResultPane";
import s from "../App.module.css";

function useSplit(key: string, initial: number) {
  const [ratio, setRatio] = useState(() => {
    try {
      return Number(localStorage.getItem(key)) || initial;
    } catch {
      return initial;
    }
  });
  const save = (r: number) => {
    setRatio(r);
    try {
      localStorage.setItem(key, String(r));
    } catch {
      /* storage unavailable; the split just won't persist */
    }
  };
  return [ratio, save] as const;
}

export function QueryPane({ tab }: { tab: Tab }) {
  const connection = useConnections((st) => st.connections.find((c) => c.id === tab.connectionId));
  const schema = useConnections((st) => (tab.connectionId ? st.live[tab.connectionId]?.schema : undefined));
  const { setSql, execute, cancel } = useTabs.getState();
  const [split, setSplit] = useSplit("kiyi.editorSplit", 0.42);
  const [dragging, setDragging] = useState(false);
  const box = useRef<HTMLDivElement>(null);

  if (!connection) return null;

  const run = async ({ sql, offset }: RunRequest) => {
    const conns = useConnections.getState();
    if (conns.live[connection.id]?.status !== "connected") await conns.activate(connection.id);
    execute(tab.id, sql, offset);
  };

  const onSplitDown = (e: React.PointerEvent) => {
    const rect = box.current?.getBoundingClientRect();
    if (!rect) return;
    setDragging(true);
    const move = (ev: PointerEvent) => setSplit(Math.min(0.85, Math.max(0.12, (ev.clientY - rect.top) / rect.height)));
    const up = () => {
      setDragging(false);
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    e.preventDefault();
  };

  const errorAt = tab.run?.error?.position != null ? tab.run.offset + tab.run.error.position - 1 : null;

  return (
    <div className={s.workspace} ref={box}>
      <div className={s.editor} style={{ height: `${split * 100}%` }}>
        <QueryEditor
          value={tab.sql}
          kind={connection.kind}
          schema={schema}
          errorAt={errorAt}
          onChange={(sql) => setSql(tab.id, sql)}
          onRun={run}
          onCancel={() => cancel(tab.id)}
        />
      </div>
      <div className={s.splitter} data-dragging={dragging || undefined} onPointerDown={onSplitDown} role="separator" aria-orientation="horizontal" />
      <div className={s.results}>
        <ResultPane run={tab.run} onCancel={() => cancel(tab.id)} />
      </div>
    </div>
  );
}
