import { useEffect, useRef, useState } from "react";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { errorMessage, ipc } from "../lib/ipc";
import type { BackupProgress, BackupReport, BackupTools, RestoreReport } from "../lib/types";
import { useActiveConnection, useConnections } from "../state/connections";
import { toast } from "../state/toasts";
import { useUi } from "../state/ui";
import { AlertIcon, ArchiveIcon, CheckIcon, CloseIcon, Spinner, StopIcon } from "./icons";
import { PromptDialog, type PromptRequest } from "./PromptDialog";
import { Button, IconButton } from "./ui";
import s from "./BackupView.module.css";

const size = (n: number) =>
  n === 0 ? "0 KB" : n < 1024 * 1024 ? `${Math.max(1, Math.round(n / 1024))} KB` : n < 1024 ** 3 ? `${(n / 1024 / 1024).toFixed(1)} MB` : `${(n / 1024 ** 3).toFixed(2)} GB`;
const today = () => new Date().toISOString().slice(0, 10);
const fmt = new Intl.NumberFormat("en-US");
const clock = (ms: number) => {
  const sec = Math.floor(ms / 1000);
  const h = Math.floor(sec / 3600);
  const mm = String(Math.floor((sec % 3600) / 60)).padStart(h ? 2 : 1, "0");
  return `${h ? `${h}:` : ""}${mm}:${String(sec % 60).padStart(2, "0")}`;
};

/** A backup or restore that's running. */
interface Job {
  kind: "backup" | "restore";
  runId: string;
  progress: BackupProgress | null;
  started: number;
  stopping: boolean;
}

/** Back up the open database to a file, or restore one into it. */
export function BackupView() {
  const connection = useActiveConnection();
  const close = () => useUi.getState().openTool(null);
  const [tools, setTools] = useState<BackupTools | null>(null);
  const [job, setJob] = useState<Job | null>(null);
  const [done, setDone] = useState<{ backup?: BackupReport; restore?: RestoreReport } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [stopped, setStopped] = useState<string | null>(null);
  const [now, setNow] = useState(Date.now());
  /** The running job's id and whether Stop was pressed, for callbacks that outlive a render. */
  const running = useRef<{ runId: string; stopping: boolean } | null>(null);
  const busy = job?.kind ?? null;
  const [prompt, setPrompt] = useState<PromptRequest | null>(null);
  const [preferKiyi, setPreferKiyi] = useState(false);

  useEffect(() => {
    if (connection) ipc.backupTools(connection.id, today()).then(setTools, (e) => setError(errorMessage(e)));
  }, [connection]);

  useEffect(() => {
    if (!job) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [job]);

  // Leaving the view stops what it started, so nothing keeps running unseen.
  useEffect(
    () => () => {
      if (running.current) void ipc.backupCancel(running.current.runId);
    },
    [],
  );

  if (!connection) return null;
  const sqlite = connection.kind === "sqlite";
  const dbName = connection.database ?? connection.name;
  const nativeTool = sqlite ? null : tools?.backup;
  const clientTools = connection.kind === "postgres" ? "pg_dump and psql (PostgreSQL client tools)" : "mysqldump and mysql (MySQL client)";
  const sqlserver = connection.kind === "sqlserver";

  /** Runs a backup or restore with progress; a stop shows as a notice rather than an error. */
  const start = async (kind: Job["kind"], work: (runId: string, onProgress: (p: BackupProgress) => void) => Promise<void>) => {
    const runId = crypto.randomUUID();
    running.current = { runId, stopping: false };
    setJob({ kind, runId, progress: null, started: Date.now(), stopping: false });
    setNow(Date.now());
    setError(null);
    setStopped(null);
    setDone(null);
    try {
      await work(runId, (progress) => setJob((j) => (j?.runId === runId ? { ...j, progress } : j)));
    } catch (e) {
      if (running.current?.stopping) setStopped(errorMessage(e));
      else setError(errorMessage(e));
    } finally {
      running.current = null;
      setJob(null);
    }
  };

  const stop = () => {
    if (!job || job.stopping || !running.current) return;
    running.current.stopping = true;
    setJob({ ...job, stopping: true });
    void ipc.backupCancel(job.runId);
  };

  const backup = async () => {
    const path = await saveDialog({ defaultPath: tools?.suggestedName ?? "backup.sql", filters: [sqlite ? { name: "SQLite database", extensions: ["db"] } : { name: "SQL", extensions: ["sql"] }] });
    if (!path) return;
    await start("backup", async (runId, onProgress) => {
      setDone({ backup: await ipc.backup(connection.id, path, preferKiyi, runId, onProgress) });
    });
  };

  const restore = async () => {
    const path = await openDialog({ multiple: false, filters: [{ name: "SQL backup", extensions: ["sql"] }, { name: "All files", extensions: ["*"] }] });
    if (typeof path !== "string") return;
    // Restoring changes the database; always confirm by name, like other irreversible actions.
    setPrompt({
      title: "Restore into this database?",
      subtitle: `${connection.name}${connection.env === "production" ? " · production" : ""}`,
      fields: [{ label: `Type ${dbName} to continue`, mono: true }],
      help: `${path.split(/[\\/]/).pop()} will be run against ${dbName}. Restore into an empty database: tables that already exist make a restore stop.`,
      action: "Restore",
      run: ([typed]) => {
        if (typed?.trim() !== dbName) throw new Error(`Type ${dbName} exactly to confirm.`);
        // The dialog closes now; progress shows in the view.
        void start("restore", async (runId, onProgress) => {
          try {
            setDone({ restore: await ipc.restore(connection.id, path, runId, onProgress) });
            toast.success("Restore finished");
          } finally {
            // A stopped or failed restore may still have changed something (MySQL, sqlcmd).
            await useConnections.getState().refreshSchema(connection.id).catch(() => {});
          }
        });
      },
    });
  };

  return (
    <div className={s.view}>
      <div className={s.toolbar}>
        <b>
          <ArchiveIcon size={15} /> Back up &amp; restore · {connection.name}
        </b>
        <IconButton label="Close" onPress={close}>
          <CloseIcon />
        </IconButton>
      </div>
      <div className={s.body}>
        <section className={s.card}>
          <h2>Back up</h2>
          <p className={s.text}>
            {sqlite
              ? "Saves a complete copy of the database file, safely even while it's in use."
              : nativeTool && !preferKiyi
                ? `Uses ${nativeTool} for a complete backup: tables, data, views, functions and triggers.`
                : "Kiyi writes the tables, their data, keys and indexes to a SQL file. Views, functions, triggers and permissions are left out."}
          </p>
          {!sqlite && !sqlserver && !nativeTool && tools && <p className={s.tip}>For a complete backup, install {clientTools}; Kiyi uses them automatically.</p>}
          {sqlserver && <p className={s.tip}>For a full SQL Server backup (views, procedures, logins), use BACKUP DATABASE or SqlPackage on the server.</p>}
          {!sqlite && nativeTool && (
            <label className={s.check}>
              <input type="checkbox" checked={preferKiyi} onChange={(e) => setPreferKiyi(e.target.checked)} /> Use Kiyi's own format instead (tables and data only)
            </label>
          )}
          {job?.kind === "backup" ? (
            <JobProgress job={job} now={now} tool={sqlite ? "SQLite" : nativeTool && !preferKiyi ? nativeTool : null} onStop={stop} />
          ) : (
            <Button variant="primary" onPress={backup} isDisabled={busy !== null || !tools}>
              Back up to a file…
            </Button>
          )}
        </section>

        <section className={s.card}>
          <h2>Restore</h2>
          <p className={s.text}>
            {sqlite
              ? "Runs a SQL script (for example from sqlite3 .dump) against this database, all or nothing. To use a backup copy of the file itself, open it as a connection."
              : `Runs a backup file against ${dbName}. Backups made by Kiyi restore anywhere${tools?.restore ? `; others use ${tools.restore}` : `; others need ${connection.kind === "postgres" ? "psql" : sqlserver ? "sqlcmd" : "the mysql client"}`}.`}
          </p>
          {connection.readOnly && <p className={s.tip}>This connection is read-only. Turn that off in its settings to restore.</p>}
          {connection.kind === "mysql" && <p className={s.tip}>MySQL can't undo structure changes if a restore stops halfway; restore into an empty database.</p>}
          {job?.kind === "restore" ? (
            <JobProgress job={job} now={now} tool={tools?.restore ?? null} onStop={stop} />
          ) : (
            <Button onPress={restore} isDisabled={busy !== null || connection.readOnly}>
              Restore from a file…
            </Button>
          )}
        </section>

        {error && (
          <div className={`${s.result} ${s.failed}`} role="alert">
            <AlertIcon size={15} />
            <span className="selectable">{error}</span>
          </div>
        )}
        {stopped && (
          <div className={`${s.result} ${s.stopped}`} role="status">
            <StopIcon size={15} />
            <span>{stopped}</span>
          </div>
        )}
        {done?.backup && (
          <div className={s.result} role="status">
            <CheckIcon size={15} />
            <span>
              Backed up {done.backup.tables} {done.backup.tables === 1 ? "table" : "tables"}
              {done.backup.rows !== null && `, ${done.backup.rows.toLocaleString("en-US")} rows`} with {done.backup.tool} ({size(done.backup.bytes)}).
              {done.backup.note && <span className={s.note}> {done.backup.note}</span>}
            </span>
          </div>
        )}
        {done?.restore && (
          <div className={s.result} role="status">
            <CheckIcon size={15} />
            <span>
              Restored with {done.restore.tool}
              {done.restore.statements !== null && ` (${done.restore.statements.toLocaleString("en-US")} statements)`}.
            </span>
          </div>
        )}
      </div>
      <PromptDialog request={prompt} onClose={() => setPrompt(null)} />
    </div>
  );
}

/** The bar, what's happening, how long it's been, and Stop. */
function JobProgress({ job, now, tool, onStop }: { job: Job; now: number; tool: string | null; onStop(): void }) {
  const p = job.progress;
  // A share only when there's something to measure against; outside tools give none.
  let share: number | null = null;
  let status = "Starting…";
  if (p && job.kind === "backup") {
    if (p.stage === "preparing") {
      status = p.tables ? `Reading table structure · ${Math.min(p.tablesDone + 1, p.tables)} of ${fmt.format(p.tables)}${p.table ? ` · ${p.table}` : ""}` : "Reading the table list…";
    } else if (p.table) {
      share = p.expectedRows ? p.rows / p.expectedRows : p.tables ? p.tablesDone / p.tables : null;
      status = `Copying ${p.table} · table ${Math.min(p.tablesDone + 1, p.tables)} of ${fmt.format(p.tables)} · ${fmt.format(p.rows)} rows · ${size(p.bytes)}`;
    } else {
      status = `${tool ?? "Backup"} is running · ${size(p.bytes)} written`;
    }
  } else if (p) {
    share = p.totalBytes ? p.bytes / p.totalBytes : null;
    status = p.totalBytes && p.rows ? `${fmt.format(p.rows)} statements run · ${size(p.bytes)} of ${size(p.totalBytes)}` : `${tool ?? "Restore"} is running…`;
  }
  if (job.stopping) status = "Stopping…";
  const pct = share === null ? null : Math.min(99, Math.max(1, Math.round(share * 100)));
  return (
    <div className={s.progress}>
      <div className={`${s.bar} ${pct === null ? s.indeterminate : ""}`} role="progressbar" aria-valuenow={pct ?? undefined} aria-valuemin={0} aria-valuemax={100}>
        <span style={pct === null ? undefined : { width: `${pct}%` }} />
      </div>
      <div className={s.status}>
        <Spinner size={13} />
        <span className={s.statusText}>{status}</span>
        <span className={s.elapsed}>
          {pct !== null && `${pct}% · `}
          {clock(now - job.started)}
        </span>
      </div>
      <Button onPress={onStop} isDisabled={job.stopping}>
        <StopIcon size={13} /> {job.stopping ? "Stopping…" : "Stop"}
      </Button>
    </div>
  );
}
