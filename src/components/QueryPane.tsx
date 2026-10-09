import { useRef, useState } from "react";
import { ipc } from "../lib/ipc";
import { prettySql, splitStatements } from "../lib/sql";
import { useConnections } from "../state/connections";
import { type Tab, useTabs } from "../state/tabs";
import { format as formatSql } from "sql-formatter";
import { errorMessage } from "../lib/ipc";
import { kbd } from "../lib/platform";
import type { ExportFormat, PlanNode, QueryExplanation } from "../lib/types";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { ContextMenu, type MenuState } from "./ContextMenu";
import { useHistory } from "../state/history";
import { toast } from "../state/toasts";
import { HistoryPanel } from "./HistoryPanel";
import { BookmarkIcon, DownloadIcon, FormatIcon, HistoryIcon, PlayIcon, SigmaIcon, SparklesIcon, Spinner, WandIcon } from "./icons";
import { AiExplainView } from "./AiExplainView";
import { ensureAi } from "../lib/askAi";
import { PlanView } from "./PlanView";
import { PromptDialog, type PromptRequest } from "./PromptDialog";
import { type EditorHandle, QueryEditor, type RunRequest } from "./QueryEditor";
import { Button } from "./ui";
import t from "./QueryPane.module.css";
import { ResultPane } from "./ResultPane";
import { ReviewDialog, type ReviewRequest } from "./ReviewDialog";
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
  const [review, setReview] = useState<ReviewRequest | null>(null);
  const editor = useRef<EditorHandle>(null);
  const [showHistory, setShowHistory] = useState(false);
  const [plan, setPlan] = useState<PlanNode | null>(null);
  const [explaining, setExplaining] = useState(false);
  const [prompt, setPrompt] = useState<PromptRequest | null>(null);
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [aiBusy, setAiBusy] = useState(false);
  const [explained, setExplained] = useState<QueryExplanation | null>(null);

  if (!connection) return null;

  const ensureConnected = async () => {
    const conns = useConnections.getState();
    if (conns.live[connection.id]?.status !== "connected") await conns.activate(connection.id);
  };

  const format = () => {
    // The selection if there is one, else the whole script.
    const req = editor.current?.target(true);
    if (!req?.sql.trim()) return;
    try {
      const formatted = formatSql(req.sql, { language: connection.kind === "mysql" ? "mysql" : connection.kind === "sqlite" ? "sqlite" : "postgresql", keywordCase: "upper", tabWidth: 2 });
      editor.current?.replace(req.offset, req.offset + req.sql.length, formatted);
    } catch (e) {
      toast.error(`Couldn't format this SQL: ${errorMessage(e)}`);
    }
  };

  const explain = async () => {
    const req = editor.current?.target(false);
    if (!req?.sql.trim()) return toast.info("Put the cursor in the query you want explained.");
    setExplaining(true);
    try {
      await ensureConnected();
      setPlan(await ipc.explain(connection.id, req.sql));
    } catch (e) {
      toast.error(`Couldn't explain this query: ${errorMessage(e)}`);
    } finally {
      setExplaining(false);
    }
  };

  const exportResults = async (format: ExportFormat) => {
    const req = editor.current?.target(false);
    if (!req?.sql.trim()) return toast.info("Put the cursor in the query whose results you want to export.");
    const path = await saveDialog({ defaultPath: `results.${format}`, filters: [{ name: format === "xlsx" ? "Excel" : format.toUpperCase(), extensions: [format] }] });
    if (!path) return;
    try {
      await ensureConnected();
      toast.info("Exporting…");
      const n = await ipc.exportQuery(connection.id, req.sql, format, path);
      toast.success(`Exported ${n.toLocaleString("en-US")} rows to ${path.split(/[\\/]/).pop()}`);
    } catch (e) {
      toast.error(`Export failed: ${errorMessage(e)}`);
    }
  };

  const today = () => new Date().toISOString().slice(0, 10);

  /** Runs an AI action with a busy state and friendly errors. */
  const withAi = async (fn: () => Promise<void>) => {
    if (!(await ensureAi())) return;
    setAiBusy(true);
    try {
      await ensureConnected();
      await fn();
    } catch (e) {
      toast.error(errorMessage(e));
    } finally {
      setAiBusy(false);
    }
  };

  const writeWithAi = () => {
    const current = editor.current?.target(true)?.sql ?? "";
    setPrompt({
      title: "Write SQL with AI",
      fields: [{ label: "What should the SQL do?", placeholder: "e.g. customers who ordered nothing this year" }],
      help: current.trim() ? "The SQL in the editor is sent along for context, with the table and column names (never your data)." : "Only table and column names are sent, never your data.",
      action: "Write",
      run: async ([instruction]) => {
        if (!instruction?.trim()) throw new Error("Describe what you want.");
        if (!(await ensureAi())) return;
        await ensureConnected();
        const r = await ipc.aiWriteSql(connection.id, instruction.trim(), current, today());
        editor.current?.insert(`-- ${instruction.trim().replace(/\s+/g, " ")}\n${prettySql(r.sql, connection.kind)}`);
        toast.success(r.writes ? `${r.explanation} It changes data: review before running.` : r.explanation);
      },
    });
  };

  const explainWithAi = () => {
    const req = editor.current?.target(false);
    if (!req?.sql.trim()) return toast.info("Put the cursor in the query you want explained.");
    withAi(async () => {
      setPlan(null);
      setExplained(await ipc.aiExplainSql(connection.id, req.sql, navigator.language.startsWith("tr") ? "Turkish" : "the language of the SQL comments, or English"));
    });
  };

  const fixWithAi = () => {
    const run = tab.run;
    if (!run?.error) return;
    // The statement that failed: the one the server named in a script, else everything that ran.
    const parts = splitStatements(run.sql, connection.kind === "mysql");
    const part = run.error.statementIndex != null ? parts[run.error.statementIndex] : parts.length === 1 ? parts[0] : null;
    const failed = (part?.text ?? run.sql).replace(/;\s*$/, "").trim();
    withAi(async () => {
      const r = await ipc.aiFixSql(connection.id, failed, run.error!.message);
      const at = tab.sql.indexOf(failed, Math.max(0, run.offset - 1));
      const fixed = failed.includes("\n") ? prettySql(r.sql, connection.kind) : r.sql;
      if (at >= 0) editor.current?.replace(at, at + failed.length, fixed);
      else editor.current?.insert(fixed);
      toast.success(`${r.explanation} Run it again to check.`);
    });
  };

  const saveQuery = () => {
    const req = editor.current?.target(true);
    if (!req?.sql.trim()) return toast.info("Write a query first.");
    const first = req.sql.trim().split("\n")[0].slice(0, 60);
    setPrompt({
      title: "Save query",
      fields: [{ label: "Name", value: tab.title.startsWith("Query ") ? first : tab.title }],
      help: "Saved queries are listed under History, for this connection.",
      action: "Save",
      run: ([name]) => {
        if (!name?.trim()) throw new Error("Give the query a name.");
        useHistory.getState().save({ name: name.trim(), sql: req.sql.trim(), connectionId: connection.id });
        useTabs.getState().patch(tab.id, { title: name.trim() });
        toast.success(`Saved “${name.trim()}”`);
      },
    });
  };

  const run = async ({ sql, offset }: RunRequest) => {
    setPlan(null);
    setExplained(null);
    await ensureConnected();
    // On production, SQL that changes anything is reviewed first, like edits made in the grid.
    if (connection.env === "production" && !connection.readOnly) {
      const check = await ipc.checkSql(connection.id, sql).catch(() => ({ writes: true, destructive: 0 }));
      if (check.writes) {
        setReview({
          title: "Run this on production?",
          subtitle: connection.name,
          summary: [
            check.destructive > 0
              ? { text: `${check.destructive === 1 ? "A statement deletes or overwrites" : `${check.destructive} statements delete or overwrite`} data. This can't be undone.`, danger: true }
              : { text: "This changes data, structure or settings on a production database." },
          ],
          statements: splitStatements(sql, connection.kind === "mysql").map((st) => st.text.replace(/;\s*$/, "")),
          action: "Run on production",
          confirmWord: connection.database ?? connection.name,
          destructive: check.destructive,
          showSql: true,
          run: async () => {
            await execute(tab.id, sql, offset);
          },
        });
        return;
      }
    }
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

  const running = tab.run?.status === "running";
  return (
    <div className={t.host}>
    <div className={s.workspace} ref={box}>
      <div className={t.toolbar}>
        <Button variant="primary" onPress={() => editor.current?.run(false)} isDisabled={running}>
          <PlayIcon size={13} /> Run <span className={t.kbd}>{kbd("↵")}</span>
        </Button>
        <Button variant="ghost" onPress={format}>
          <FormatIcon size={14} /> Format
        </Button>
        <Button variant="ghost" onPress={explain} isDisabled={explaining}>
          {explaining ? <Spinner size={13} /> : <SigmaIcon size={14} />} Explain
        </Button>
        <Button
          variant="ghost"
          isDisabled={aiBusy}
          onPress={() => {
            const r = document.activeElement?.getBoundingClientRect();
            setMenu({
              x: r?.left ?? 0,
              y: (r?.bottom ?? 0) + 4,
              items: [
                { label: "Write SQL from a description…", onSelect: writeWithAi },
                { label: "Explain this query in plain words", onSelect: explainWithAi },
                { label: "Fix the last error", onSelect: fixWithAi, disabled: !tab.run?.error },
              ],
            });
          }}
        >
          {aiBusy ? <Spinner size={13} /> : <WandIcon size={14} />} AI
        </Button>
        <span className={t.spacer} />
        <Button
          variant="ghost"
          onPress={() => {
            const r = document.activeElement?.getBoundingClientRect();
            setMenu({
              x: r?.left ?? 0,
              y: (r?.bottom ?? 0) + 4,
              items: [
                { label: "Export results to Excel…", onSelect: () => exportResults("xlsx") },
                { label: "Export results to CSV…", onSelect: () => exportResults("csv") },
                { label: "Export results to JSON…", onSelect: () => exportResults("json") },
              ],
            });
          }}
        >
          <DownloadIcon size={14} /> Export
        </Button>
        <Button variant="ghost" onPress={saveQuery}>
          <BookmarkIcon size={14} /> Save
        </Button>
        <Button variant="ghost" onPress={() => setShowHistory((v) => !v)} aria-pressed={showHistory}>
          <HistoryIcon size={14} /> History
        </Button>
      </div>
      <div className={s.editor} style={{ height: `${split * 100}%` }}>
        <QueryEditor
          ref={editor}
          value={tab.sql}
          kind={connection.kind}
          schema={schema}
          errorAt={errorAt}
          onChange={(sql) => {
            setSql(tab.id, sql);
            // Once the AI's query is edited, its question and chart no longer describe it.
            if (tab.ai && !sql.includes(tab.ai.question.replace(/\s+/g, " "))) useTabs.getState().patch(tab.id, { ai: undefined });
          }}
          onRun={run}
          onCancel={() => cancel(tab.id)}
        />
      </div>
      <div className={s.splitter} data-dragging={dragging || undefined} onPointerDown={onSplitDown} role="separator" aria-orientation="horizontal" />
      <div className={s.results}>
        {tab.ai && (
          <div className={t.aiBanner}>
            <SparklesIcon size={13} />
            <span>
              <b>{tab.ai.question}</b> · {tab.ai.explanation}
            </span>
          </div>
        )}
        {plan ? (
          <PlanView plan={plan} onClose={() => setPlan(null)} />
        ) : explained ? (
          <AiExplainView explanation={explained} onClose={() => setExplained(null)} />
        ) : (
          <ResultPane run={tab.run} kind={connection.kind} hint={tab.ai?.chart} onCancel={() => cancel(tab.id)} onFix={tab.run?.error ? fixWithAi : undefined} />
        )}
      </div>
      <ReviewDialog request={review} kind={connection.kind} env={connection.env} onClose={() => setReview(null)} />
      <PromptDialog request={prompt} onClose={() => setPrompt(null)} />
      <ContextMenu menu={menu} onClose={() => setMenu(null)} />
    </div>
    {showHistory && <HistoryPanel connectionId={connection.id} onPick={(sql) => editor.current?.insert(sql)} onClose={() => setShowHistory(false)} />}
    </div>
  );
}
