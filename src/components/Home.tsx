import { useEffect, useMemo, useState } from "react";
import { askQuestion } from "../lib/askAi";
import { exportConnections, importConnections, openSample } from "../lib/connectionFiles";
import { ipc } from "../lib/ipc";
import type { DriverInfo, LocalDatabase } from "../lib/types";
import { connectionWhere, driverFor, useCatalog } from "../state/catalog";
import { useActiveConnection, useConnections } from "../state/connections";
import { useSettings } from "../state/settings";
import { useTabs } from "../state/tabs";
import { useUi } from "../state/ui";
import { ArchiveIcon, ArrowIcon, CompareIcon, DatabaseIcon, DiagramIcon, DownloadIcon, MoveIcon, ObjectsIcon, SampleIcon, UploadIcon, LinkIcon, PlusIcon, RefreshIcon, SearchIcon, SparklesIcon, Spinner, TableIcon, ViewIcon } from "./icons";
import { Button } from "./ui";
import s from "./Home.module.css";

const ENV_LABEL = { local: "Local", staging: "Staging", production: "Production" } as const;
const fmtRows = (n: number) => new Intl.NumberFormat("en-US", { notation: n >= 10_000 ? "compact" : "standard", maximumFractionDigits: 1 }).format(n);

/** What the main area shows when a connection is open but no tab is. */
export function Overview() {
  const connection = useActiveConnection();
  const live = useConnections((st) => (connection ? st.live[connection.id] : undefined));
  const drivers = useCatalog((st) => st.drivers);
  const developerMode = useSettings((st) => st.developerMode);
  const { openTable, openCreate } = useTabs.getState();
  const [filter, setFilter] = useState("");

  const snapshot = live?.schema;
  const items = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    const multi = (snapshot?.schemas.length ?? 0) > 1;
    return (snapshot?.schemas ?? []).flatMap((sc) =>
      sc.tables
        .filter((t) => !needle || t.name.toLowerCase().includes(needle))
        .map((t) => ({ schema: sc.name, table: t, label: multi && sc.name !== snapshot?.defaultSchema ? `${sc.name}.${t.name}` : t.name })),
    );
  }, [snapshot, filter]);

  if (!connection) return null;
  const tables = items.filter((i) => i.table.kind === "table");
  const views = items.filter((i) => i.table.kind === "view");
  const defaultSchema = snapshot?.defaultSchema ?? snapshot?.schemas[0]?.name ?? null;

  const card = ({ schema, table, label }: (typeof items)[number]) => (
    <button key={`${schema}.${table.name}`} className={s.card} onClick={() => openTable(connection.id, schema, table.name)}>
      <span className={s.cardTop}>{table.kind === "view" ? <ViewIcon /> : <TableIcon />}</span>
      <span className={s.cardName}>{label}</span>
      <span className={s.cardMeta}>
        {table.rowEstimate !== null ? `~${fmtRows(table.rowEstimate)} rows · ` : ""}
        {table.columns.length} columns
      </span>
      <span className={s.cardCols}>{table.columns.slice(0, 4).map((c) => c.name).join(" · ")}</span>
    </button>
  );

  return (
    <div className={s.page}>
      <div className={s.header}>
        <div>
          <h1 className={s.title}>{connection.name}</h1>
          <p className={s.meta}>
            <span className={s.env}>{ENV_LABEL[connection.env]}</span>
            <span>{developerMode && live?.serverVersion ? live.serverVersion : driverFor(connection, drivers)?.name}</span>
            <span>·</span>
            <span>
              {tables.length} {tables.length === 1 ? "table" : "tables"}{views.length ? `, ${views.length} ${views.length === 1 ? "view" : "views"}` : ""}
            </span>
            {connection.readOnly && <span>· read-only</span>}
          </p>
        </div>
        <div className={s.headerActions}>
          <Button variant="ghost" onPress={() => useUi.getState().openTool("diagram")}>
            <DiagramIcon size={14} /> Diagram
          </Button>
          <Button variant="ghost" onPress={() => useUi.getState().openTool("objects")}>
            <ObjectsIcon size={14} /> Objects
          </Button>
          <Button variant="ghost" onPress={() => useUi.getState().openTool("migrate")}>
            <MoveIcon size={14} /> Move data
          </Button>
          <Button variant="ghost" onPress={() => useUi.getState().openTool("compare")}>
            <CompareIcon size={14} /> Compare
          </Button>
          <Button variant="ghost" onPress={() => useUi.getState().openTool("backup")}>
            <ArchiveIcon size={14} /> Back up
          </Button>
          <label className={s.search}>
            <SearchIcon size={14} />
            <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Find a table" spellCheck={false} />
          </label>
        </div>
      </div>

      <AskBox connectionId={connection.id} />

      <div className={s.grid}>
        {tables.map(card)}
        {!connection.readOnly && !filter && (
          <button className={`${s.card} ${s.newCard}`} onClick={() => openCreate(connection.id, defaultSchema)}>
            <PlusIcon size={18} />
            New table
          </button>
        )}
      </div>
      {views.length > 0 && (
        <>
          <h2 className={s.sectionTitle}>Views</h2>
          <div className={s.grid}>{views.map(card)}</div>
        </>
      )}
    </div>
  );
}

/** "Ask a question about your data": AI writes the query, it runs, and the answer shows as a chart. */
function AskBox({ connectionId }: { connectionId: string }) {
  const [question, setQuestion] = useState("");
  const [busy, setBusy] = useState(false);
  const examples = ["How many rows were added each month?", "Top 10 by total", "Son 30 günde kaç kayıt eklendi?"];
  const ask = async (q: string) => {
    setBusy(true);
    if (await askQuestion(connectionId, q)) setQuestion("");
    setBusy(false);
  };
  return (
    <form
      className={s.ask}
      onSubmit={(e) => {
        e.preventDefault();
        if (question.trim() && !busy) ask(question);
      }}
    >
      <SparklesIcon size={16} />
      <input value={question} onChange={(e) => setQuestion(e.target.value)} placeholder="Ask a question about your data, in any language…" spellCheck={false} aria-label="Ask a question" disabled={busy} />
      <Button type="submit" variant="primary" isDisabled={!question.trim() || busy}>
        {busy ? <Spinner size={13} /> : null} Ask
      </Button>
      <div className={s.examples}>
        {examples.map((x) => (
          <button key={x} type="button" onClick={() => setQuestion(x)}>
            {x}
          </button>
        ))}
      </div>
    </form>
  );
}

/** Saved connections, when none is open. */
export function ConnectionsHome({ onNew }: { onNew(): void }) {
  const connections = useConnections((st) => st.connections);
  const live = useConnections((st) => st.live);
  const activate = useConnections((st) => st.activate);
  const drivers = useCatalog((st) => st.drivers);
  return (
    <div className={s.page}>
      <div className={s.header}>
        <div>
          <h1 className={s.title}>Connections</h1>
          <p className={s.meta}>Open a database to browse and edit its tables.</p>
        </div>
        <div className={s.headerActions}>
          <Button variant="ghost" onPress={() => importConnections()}>
            <UploadIcon size={14} /> Import
          </Button>
          <Button variant="ghost" isDisabled={connections.length === 0} onPress={() => exportConnections()}>
            <DownloadIcon size={14} /> Export
          </Button>
        </div>
      </div>
      <div className={s.grid}>
        {connections.map((c) => (
          <button key={c.id} className={s.card} onClick={() => activate(c.id)}>
            <span className={s.cardTop}>
              <DatabaseIcon />
              <span className={s.envTag} data-env={c.env}>
                {ENV_LABEL[c.env]}
              </span>
            </span>
            <span className={s.cardName}>{c.name}</span>
            <span className={s.cardMeta}>
              {driverFor(c, drivers)?.name} · {connectionWhere(c)}
            </span>
            {live[c.id]?.status === "error" && <span className={s.cardError}>{live[c.id]?.error}</span>}
          </button>
        ))}
        <button className={`${s.card} ${s.newCard}`} onClick={onNew}>
          <PlusIcon size={18} />
          New connection
        </button>
      </div>
    </div>
  );
}

export type ConnectInit = { url?: string; driver?: DriverInfo; host?: string; port?: number };

/** First run: databases found on this Mac, a URL box, or set one up by hand. */
export function Welcome({ onConnect }: { onConnect(init: ConnectInit): void }) {
  const drivers = useCatalog((st) => st.drivers);
  const openSettings = useUi((st) => st.openSettings);
  const [url, setUrl] = useState("");
  const [found, setFound] = useState<LocalDatabase[] | null>(null);

  const scan = () => {
    setFound(null);
    ipc.discoverLocal().then(setFound, () => setFound([]));
  };
  useEffect(scan, []);

  return (
    <div className={s.welcome}>
      <div className={s.welcomeInner}>
        <img src="/icon.svg" alt="" className={s.mark} />
        <h1 className={s.headline}>Welcome to Kiyi</h1>
        <p className={s.lede}>Connect a database to browse, search and edit its data like a spreadsheet. No SQL needed.</p>

        <button className={s.sample} onClick={() => openSample()}>
          <SampleIcon size={20} />
          <span className={s.foundMain}>
            <span className={s.foundName}>Try the sample database</span>
            <span className={s.sampleText}>A small store with customers, products and orders to explore. Nothing to install.</span>
          </span>
          <ArrowIcon size={16} />
        </button>

        <div className={s.panel}>
          <div className={s.panelHead}>
            <span className={s.panelTitle}>Found on this computer</span>
            <button className={s.rescan} onClick={scan} disabled={found === null}>
              {found === null ? <Spinner size={12} /> : <RefreshIcon size={13} />}
              {found === null ? "Looking…" : "Look again"}
            </button>
          </div>
          {found === null && <p className={s.panelEmpty}>Looking for databases running on this computer…</p>}
          {found?.length === 0 && <p className={s.panelEmpty}>No local databases found. Paste a connection URL below, or set one up by hand.</p>}
          {found?.map((db) => {
            const driver = drivers.find((d) => d.id === db.driver);
            return (
              <div key={db.port} className={s.found}>
                <DatabaseIcon size={18} />
                <span className={s.foundMain}>
                  <span className={s.foundName}>
                    {driver?.name ?? db.driver}
                    {db.version && <span className={s.foundVersion}>{db.version}</span>}
                  </span>
                  <span className={s.foundAddr}>
                    {db.host}:{db.port}
                    {db.container && ` · Docker: ${db.container}`}
                  </span>
                </span>
                <Button variant="primary" onPress={() => onConnect({ driver, host: db.host, port: db.port })}>
                  Connect
                </Button>
              </div>
            );
          })}
        </div>

        <form
          className={s.paste}
          onSubmit={(e) => {
            e.preventDefault();
            if (url.trim()) onConnect({ url });
          }}
        >
          <LinkIcon size={16} />
          <input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="…or paste a connection URL, e.g. postgres://user:pass@host/db" spellCheck={false} aria-label="Connection URL" />
          <Button type="submit" variant="primary" isDisabled={!url.trim()}>
            Connect
          </Button>
        </form>

        <p className={s.or}>Set up a connection by hand</p>
        <div className={s.drivers}>
          {drivers.map((d) => (
            <button key={d.id} className={s.driver} disabled={!d.kind} onClick={() => onConnect({ driver: d })}>
              <DatabaseIcon size={20} />
              {d.name}
              {!d.kind && <span className={s.soon}>Coming soon</span>}
            </button>
          ))}
        </div>

        <div className={s.hints}>
          <button className={s.aiHint} onClick={() => openSettings("ai")}>
            <SparklesIcon size={14} /> Want to ask for rows in plain words? Set up AI (optional)
          </button>
          <button className={s.aiHint} onClick={() => importConnections()}>
            <UploadIcon size={14} /> Import connections from another computer
          </button>
        </div>
      </div>
    </div>
  );
}
