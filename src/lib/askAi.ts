import { useConnections } from "../state/connections";
import { useTabs } from "../state/tabs";
import { toast } from "../state/toasts";
import { useUi } from "../state/ui";
import { errorMessage, ipc } from "./ipc";
import { prettySql } from "./sql";

const today = () => new Date().toISOString().slice(0, 10);

/** True when an AI provider is ready; otherwise explains and opens Settings → AI. */
export async function ensureAi(): Promise<boolean> {
  const ai = await ipc.aiStatus().catch(() => null);
  if (ai?.configured) return true;
  toast.info(ai?.provider ? `Add an API key for ${ai.provider} to use AI.` : "Choose an AI provider to use AI.");
  useUi.getState().openSettings("ai");
  return false;
}

/**
 * Answers a question about the data: AI writes one read-only query, which opens in a new SQL tab
 * (so the SQL is always visible), runs, and shows its result as a chart when that fits.
 */
export async function askQuestion(connectionId: string, question: string): Promise<boolean> {
  const q = question.trim();
  if (!q || !(await ensureAi())) return false;
  const conns = useConnections.getState();
  if (conns.live[connectionId]?.status !== "connected") await conns.activate(connectionId);
  try {
    const r = await ipc.aiAsk(connectionId, q, today());
    const kind = conns.connections.find((c) => c.id === connectionId)?.kind ?? "postgres";
    const query = prettySql(r.sql, kind);
    const comment = `-- ${q.replace(/\s+/g, " ")}\n`;
    const sql = `${comment}${query}`;
    const tabs = useTabs.getState();
    const id = tabs.open({ connectionId, sql, title: q.length > 32 ? `${q.slice(0, 31)}…` : q, ai: { question: q, explanation: r.explanation, chart: r.chart } });
    await tabs.execute(id, query, comment.length);
    return true;
  } catch (e) {
    toast.error(errorMessage(e));
    return false;
  }
}
