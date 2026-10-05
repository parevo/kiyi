import { useMemo, useState } from "react";
import type { DriverInfo } from "../lib/types";
import { driverFor, useCatalog } from "../state/catalog";
import { useActiveConnection, useConnections } from "../state/connections";
import { useSettings } from "../state/settings";
import { useTabs } from "../state/tabs";
import { DatabaseIcon, PlusIcon, SearchIcon, TableIcon, ViewIcon } from "./icons";
import { Button } from "./ui";
import s from "./Home.module.css";

const ENV_LABEL = { local: "Yerel", staging: "Test", production: "Canlı" } as const;

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
      <span className={s.cardMeta}>{table.columns.length} alan</span>
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
              {tables.length} tablo{views.length ? `, ${views.length} görünüm` : ""}
            </span>
            {connection.readOnly && <span>· salt okunur</span>}
          </p>
        </div>
        <label className={s.search}>
          <SearchIcon size={14} />
          <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Tablolarda ara" spellCheck={false} />
        </label>
      </div>

      <div className={s.grid}>
        {tables.map(card)}
        {!connection.readOnly && !filter && (
          <button className={`${s.card} ${s.newCard}`} onClick={() => openCreate(connection.id, defaultSchema)}>
            <PlusIcon size={18} />
            Yeni tablo
          </button>
        )}
      </div>
      {views.length > 0 && (
        <>
          <h2 className={s.sectionTitle}>Görünümler</h2>
          <div className={s.grid}>{views.map(card)}</div>
        </>
      )}
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
      <h1 className={s.headline}>Kıyı'ya hoş geldin</h1>
      <p className={s.lede}>Veritabanına bağlan, kayıtlarını bir tablo gibi gör ve düzenle. SQL bilmene gerek yok.</p>
      <form
        className={s.paste}
        onSubmit={(e) => {
          e.preventDefault();
          onConnect({ url });
        }}
      >
        <DatabaseIcon />
        <input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="Bağlantı adresini yapıştır" spellCheck={false} autoFocus aria-label="Bağlantı adresi" />
        <Button type="submit" variant="primary" isDisabled={!url.trim()}>
          Bağlan
        </Button>
      </form>
      <p className={s.or}>ya da veritabanını seç</p>
      <div className={s.drivers}>
        {drivers.map((d) => (
          <button key={d.id} className={s.driver} disabled={!d.kind} onClick={() => onConnect({ driver: d })}>
            <span className={s.driverMark}>{d.name.slice(0, 2)}</span>
            {d.name}
            {!d.kind && <span className={s.soon}>Yakında</span>}
          </button>
        ))}
      </div>
    </div>
  );
}
