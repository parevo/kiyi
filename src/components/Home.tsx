import { useMemo, useState } from "react";
import type { DriverInfo } from "../lib/types";
import { driverFor, useCatalog } from "../state/catalog";
import { useActiveConnection, useConnections } from "../state/connections";
import { useSettings } from "../state/settings";
import { useTabs } from "../state/tabs";
import { DatabaseIcon, PlusIcon, SearchIcon, TableIcon, ViewIcon } from "./icons";
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
        <label className={s.search}>
          <SearchIcon size={14} />
          <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Find a table" spellCheck={false} />
        </label>
      </div>

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
              {driverFor(c, drivers)?.name} · {c.host}
              {c.database ? ` / ${c.database}` : ""}
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

/** First run: paste an address, or pick a database. */
export function Welcome({ onConnect }: { onConnect(init: { url?: string; driver?: DriverInfo }): void }) {
  const drivers = useCatalog((st) => st.drivers);
  const [url, setUrl] = useState("");
  return (
    <div className={s.welcome}>
      <img src="/icon.svg" alt="" className={s.mark} />
      <h1 className={s.headline}>Welcome to Kiyi</h1>
      <p className={s.lede}>Connect to a database, then browse and edit its data like a spreadsheet. No SQL needed.</p>
      <form
        className={s.paste}
        onSubmit={(e) => {
          e.preventDefault();
          onConnect({ url });
        }}
      >
        <DatabaseIcon />
        <input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="Paste a connection URL" spellCheck={false} autoFocus aria-label="Connection URL" />
        <Button type="submit" variant="primary" isDisabled={!url.trim()}>
          Connect
        </Button>
      </form>
      <p className={s.or}>or choose your database</p>
      <div className={s.drivers}>
        {drivers.map((d) => (
          <button key={d.id} className={s.driver} disabled={!d.kind} onClick={() => onConnect({ driver: d })}>
            <DatabaseIcon size={22} />
            {d.name}
            {!d.kind && <span className={s.soon}>Coming soon</span>}
          </button>
        ))}
      </div>
    </div>
  );
}
