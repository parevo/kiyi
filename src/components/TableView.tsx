import { useCallback, useEffect, useRef, useState } from "react";
import { errorMessage, ipc } from "../lib/ipc";
import type { TableDetails } from "../lib/types";
import { useConnections } from "../state/connections";
import { type Tab, tableKey, useTabs } from "../state/tabs";
import { PlusIcon, RefreshIcon, SearchIcon, Spinner } from "./icons";
import { ReviewDialog, type ReviewRequest } from "./ReviewDialog";
import { StructureEditor, type StructureHandle } from "./StructureEditor";
import { TableData, type TableDataHandle, type TableDataStatus } from "./TableData";
import { Button, IconButton } from "./ui";
import s from "./TableView.module.css";

const fmt = new Intl.NumberFormat("tr-TR");
const compact = new Intl.NumberFormat("tr-TR", { notation: "compact", maximumFractionDigits: 1 });

/** A table tab (data + structure) or a "new table" tab. */
export function TableView({ tab, active, onOpenSql }: { tab: Tab; active: boolean; onOpenSql(sql: string): void }) {
  const connection = useConnections((st) => st.connections.find((c) => c.id === tab.connectionId));
  const snapshot = useConnections((st) => (tab.connectionId ? st.live[tab.connectionId]?.schema : undefined));
  const refreshSchema = useConnections((st) => st.refreshSchema);
  const patchTab = useTabs((st) => st.patch);

  const creating = tab.kind === "create";
  const view = creating ? "structure" : (tab.view ?? "data");
  const schema = tab.schema ?? null;

  const [details, setDetails] = useState<TableDetails | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [review, setReview] = useState<ReviewRequest | null>(null);
  const [dataStatus, setDataStatus] = useState<TableDataStatus | null>(null);
  const [structChanges, setStructChanges] = useState(0);
  const [visitedStructure, setVisitedStructure] = useState(view === "structure");
  const data = useRef<TableDataHandle>(null);
  const structure = useRef<StructureHandle>(null);

  const loadDetails = useCallback(async () => {
    if (creating || !tab.connectionId || !tab.tableName) return;
    setLoadError(null);
    try {
      setDetails(await ipc.tableDetails(tab.connectionId, schema, tab.tableName));
    } catch (e) {
      setLoadError(errorMessage(e));
    }
  }, [creating, tab.connectionId, tab.tableName, schema]);

  useEffect(() => {
    loadDetails();
  }, [loadDetails]);

  useEffect(() => {
    if (view === "structure") setVisitedStructure(true);
  }, [view]);

  const dirty = (dataStatus?.pending ?? 0) > 0 || structChanges > 0;
  useEffect(() => {
    if (!!tab.dirty !== dirty) patchTab(tab.id, { dirty });
  }, [dirty, tab.dirty, tab.id, patchTab]);

  const onApplied = async (name: string) => {
    if (!tab.connectionId) return;
    await refreshSchema(tab.connectionId);
    if (creating || name !== tab.tableName) {
      patchTab(tab.id, {
        kind: "table",
        table: tableKey(tab.connectionId, schema, name),
        tableName: name,
        title: name,
        view: creating ? "data" : view,
      });
    } else {
      await loadDetails();
    }
  };

  if (!connection) return null;

  const setView = (v: "data" | "structure") => patchTab(tab.id, { view: v });
  const loaded = dataStatus?.loaded ?? 0;
  const total = dataStatus?.total;

  return (
    <div className={s.view}>
      <div className={s.toolbar}>
        {!creating && (
          <div className={s.toggle} role="group" aria-label="Görünüm">
            <button aria-pressed={view === "data"} onClick={() => setView("data")}>
              Veri
            </button>
            <button aria-pressed={view === "structure"} onClick={() => setView("structure")}>
              Yapı
            </button>
          </div>
        )}

        {view === "data" && dataStatus && (
          <>
            <span className={s.sep} />
            <Button variant="ghost" onPress={() => data.current?.toggleFilters()}>
              <SearchIcon size={14} /> Filtre
              {dataStatus.filters > 0 && <span className={s.badge}>{dataStatus.filters}</span>}
            </Button>
            <IconButton label="Yenile" shortcut="⌘R" onPress={() => data.current?.refresh()}>
              {dataStatus.loading ? <Spinner size={13} /> : <RefreshIcon size={14} />}
            </IconButton>
            {!connection.readOnly && !details?.isView && (
              <>
                <span className={s.sep} />
                <Button variant="ghost" onPress={() => data.current?.addRow()}>
                  <PlusIcon size={14} /> Satır
                </Button>
                {dataStatus.selectedRows > 0 && (
                  <Button variant="ghost" onPress={() => data.current?.deleteSelected()}>
                    {dataStatus.selectedRows} satırı sil
                  </Button>
                )}
              </>
            )}
          </>
        )}

        <span className={s.spacer} />

        {view === "data" && (dataStatus?.pending ?? 0) > 0 && (
          <>
            <span className={s.pending}>{dataStatus!.pending} değişiklik</span>
            <Button variant="ghost" onPress={() => data.current?.discard()}>
              Geri al
            </Button>
            <Button variant="primary" onPress={() => data.current?.save()}>
              Kaydet <span className={s.kbd}>⌘S</span>
            </Button>
          </>
        )}
        {view === "structure" && !creating && structChanges > 0 && (
          <>
            <span className={s.pending}>{structChanges} değişiklik</span>
            <Button variant="ghost" onPress={() => structure.current?.discard()}>
              Geri al
            </Button>
            <Button variant="primary" onPress={() => structure.current?.save()}>
              Önizle ve uygula <span className={s.kbd}>⌘S</span>
            </Button>
          </>
        )}
      </div>

      <div className={s.body}>
        {loadError && (
          <div className={s.error} role="alert">
            <b>Tablo bilgisi okunamadı</b>
            <span className="selectable">{loadError}</span>
          </div>
        )}
        {!creating && !details && !loadError && <div className={s.center}>Yükleniyor…</div>}

        {details && (
          <div style={{ display: view === "data" ? "contents" : "none" }}>
            <TableData
              ref={data}
              tabId={tab.id}
              connection={connection}
              details={details}
              active={active && view === "data"}
              onStatus={setDataStatus}
              onReview={setReview}
            />
          </div>
        )}
        {(creating || (details && visitedStructure)) && (
          <div style={{ display: view === "structure" ? "contents" : "none" }}>
            <StructureEditor
              ref={structure}
              connection={connection}
              schema={schema}
              details={details}
              snapshot={snapshot}
              active={active && view === "structure"}
              onStatus={(st) => setStructChanges(st.changes)}
              onReview={setReview}
              onApplied={onApplied}
            />
          </div>
        )}
      </div>

      {view === "data" && dataStatus && (
        <div className={s.status}>
          <span>
            {fmt.format(loaded)}
            {total !== null && total !== undefined && total !== loaded && (
              <span className={s.faint}> / {dataStatus.totalIsEstimate ? `~${compact.format(total)}` : fmt.format(total)}</span>
            )}{" "}
            satır
          </span>
          {dataStatus.elapsedMs !== null && <span className={s.faint}>{Math.round(dataStatus.elapsedMs)} ms</span>}
          {connection.readOnly && <span className={s.faint}>salt okunur</span>}
          <span className={s.spacer} />
          {dataStatus.sql && (
            <button className={s.link} onClick={() => onOpenSql(dataStatus.sql!)} title="Bu sorguyu editörde aç">
              SQL
            </button>
          )}
        </div>
      )}

      <ReviewDialog
        request={review}
        kind={connection.kind}
        env={connection.env}
        onClose={() => setReview(null)}
        onOpenInEditor={onOpenSql}
      />
    </div>
  );
}
