import { useEffect, useState } from "react";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { errorMessage, ipc } from "../lib/ipc";
import type { BackupReport, BackupTools, RestoreReport } from "../lib/types";
import { useActiveConnection, useConnections } from "../state/connections";
import { toast } from "../state/toasts";
import { useUi } from "../state/ui";
import { AlertIcon, ArchiveIcon, CheckIcon, CloseIcon, Spinner } from "./icons";
import { PromptDialog, type PromptRequest } from "./PromptDialog";
import { Button, IconButton } from "./ui";
import s from "./BackupView.module.css";

const size = (n: number) => (n < 1024 * 1024 ? `${Math.max(1, Math.round(n / 1024))} KB` : `${(n / 1024 / 1024).toFixed(1)} MB`);
const today = () => new Date().toISOString().slice(0, 10);

/** Back up the open database to a file, or restore one into it. */
export function BackupView() {
  const connection = useActiveConnection();
  const close = () => useUi.getState().openTool(null);
  const [tools, setTools] = useState<BackupTools | null>(null);
  const [busy, setBusy] = useState<"backup" | "restore" | null>(null);
  const [done, setDone] = useState<{ backup?: BackupReport; restore?: RestoreReport } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [prompt, setPrompt] = useState<PromptRequest | null>(null);
  const [preferKiyi, setPreferKiyi] = useState(false);

  useEffect(() => {
    if (connection) ipc.backupTools(connection.id, today()).then(setTools, (e) => setError(errorMessage(e)));
  }, [connection]);

  if (!connection) return null;
  const sqlite = connection.kind === "sqlite";
  const dbName = connection.database ?? connection.name;
  const nativeTool = sqlite ? null : tools?.backup;
  const clientTools = connection.kind === "postgres" ? "pg_dump and psql (PostgreSQL client tools)" : "mysqldump and mysql (MySQL client)";
  const sqlserver = connection.kind === "sqlserver";

  const backup = async () => {
    const path = await saveDialog({ defaultPath: tools?.suggestedName ?? "backup.sql", filters: [sqlite ? { name: "SQLite database", extensions: ["db"] } : { name: "SQL", extensions: ["sql"] }] });
    if (!path) return;
    setBusy("backup");
    setError(null);
    setDone(null);
    try {
      setDone({ backup: await ipc.backup(connection.id, path, preferKiyi) });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(null);
    }
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
      run: async ([typed]) => {
        if (typed?.trim() !== dbName) throw new Error(`Type ${dbName} exactly to confirm.`);
        setBusy("restore");
        setError(null);
        setDone(null);
        try {
          setDone({ restore: await ipc.restore(connection.id, path) });
          await useConnections.getState().refreshSchema(connection.id);
          toast.success("Restore finished");
        } catch (e) {
          setError(errorMessage(e));
        } finally {
          setBusy(null);
        }
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
          <Button variant="primary" onPress={backup} isDisabled={busy !== null || !tools}>
            {busy === "backup" ? <Spinner size={13} /> : null} {busy === "backup" ? "Backing up…" : "Back up to a file…"}
          </Button>
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
          <Button onPress={restore} isDisabled={busy !== null || connection.readOnly}>
            {busy === "restore" ? <Spinner size={13} /> : null} {busy === "restore" ? "Restoring…" : "Restore from a file…"}
          </Button>
        </section>

        {error && (
          <div className={`${s.result} ${s.failed}`} role="alert">
            <AlertIcon size={15} />
            <span className="selectable">{error}</span>
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
