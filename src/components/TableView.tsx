import { useCallback, useEffect, useRef, useState } from "react";
import { errorMessage, ipc } from "../lib/ipc";
import type { Cell, ForeignKeyDesign, TableDetails } from "../lib/types";
import { useConnections } from "../state/connections";
import { useSettings } from "../state/settings";
import { type Tab, tableKey, useTabs } from "../state/tabs";
import { toast } from "../state/toasts";
import { save as saveDialog, open as openDialog } from "@tauri-apps/plugin-dialog";
import { ContextMenu, type MenuState } from "./ContextMenu";
import { CopyIcon, MoreIcon, PanelIcon, PlusIcon, RefreshIcon, Spinner, TrashIcon } from "./icons";
import { ImportSheet } from "./ImportSheet";
import type { ExportFormat } from "../lib/types";
import { InsertRowSheet } from "./InsertRowSheet";
import { ActiveFilters, FilterButton, SearchBar, SortButton } from "./QueryControls";
import { useUi } from "../state/ui";
import { ReviewDialog, type ReviewRequest } from "./ReviewDialog";
import { CreateTable, StructureView } from "./StructureEditor";
import { emptyQuery, TableData, type TableDataHandle, type TableDataStatus, type TableQuery } from "./TableData";
import { Button, IconButton } from "./ui";
import s from "./TableView.module.css";
import { kbd } from "../lib/platform";

const fmt = new Intl.NumberFormat("en-US");
const compact = new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 1 });
const today = () => new Date().toISOString().slice(0, 10);

/** A table tab (data + structure) or a "new table" tab. */
export function TableView({ tab, active, onOpenSql }: { tab: Tab; active: boolean; onOpenSql(sql: string): void }) {
  const connection = useConnections((st) => st.connections.find((c) => c.id === tab.connectionId));
  const snapshot = useConnections((st) => (tab.connectionId ? st.live[tab.connectionId]?.schema : undefined));
  const refreshSchema = useConnections((st) => st.refreshSchema);
  const patchTab = useTabs((st) => st.patch);
  const openTable = useTabs((st) => st.openTable);
  const developerMode = useSettings((st) => st.developerMode);
  const inspectorOpen = useSettings((st) => st.inspectorOpen);
  const setSettings = useSettings((st) => st.set);

  const creating = tab.kind === "create";
  const view = tab.view ?? "data";
  const schema = tab.schema ?? null;

  const [details, setDetails] = useState<TableDetails | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [query, setQuery] = useState<TableQuery>(() => emptyQuery(tab.initialFilters ?? []));
  const [status, setStatus] = useState<TableDataStatus | null>(null);
  const [review, setReview] = useState<ReviewRequest | null>(null);
  const [insert, setInsert] = useState<{ prefill?: Record<string, Cell> } | null>(null);
  const [asking, setAsking] = useState(false);
  const data = useRef<TableDataHandle>(null);
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [importPath, setImportPath] = useState<string | null>(null);

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

  const dirty = (status?.pending ?? 0) > 0;
  useEffect(() => {
    if (!!tab.dirty !== dirty) patchTab(tab.id, { dirty });
  }, [dirty, tab.dirty, tab.id, patchTab]);

  if (!connection) return null;

  if (creating) {
    return (
      <CreateTable
        connection={connection}
        schema={schema}
        snapshot={snapshot}
        onCreated={async (name) => {
          await refreshSchema(connection.id);
          patchTab(tab.id, { kind: "table", table: tableKey(connection.id, schema, name), tableName: name, title: name, view: "data" });
        }}
        onReview={setReview}
        review={<ReviewDialog request={review} kind={connection.kind} env={connection.env} onClose={() => setReview(null)} onOpenInEditor={onOpenSql} />}
      />
    );
  }

  const setView = (v: "data" | "structure") => patchTab(tab.id, { view: v });
  const editable = !!details && !details.isView && !connection.readOnly;

  const ask = async (prompt: string) => {
    if (!details) return;
    const ai = await ipc.aiStatus().catch(() => null);
    if (!ai?.configured) {
      toast.info(ai?.provider ? `Add an API key for ${ai.provider} to use Ask AI.` : "Choose an AI provider to use Ask AI.");
      return useUi.getState().openSettings("ai");
    }
    setAsking(true);
    try {
      const r = await ipc.aiFilters(connection.id, schema, details.design.name, prompt, today());
      setQuery({ filters: r.filters, sort: r.sort[0] ?? null, rawWhere: r.condition, search: null, ai: { prompt, explanation: r.explanation } });
    } catch (e) {
      toast.error(errorMessage(e));
    } finally {
      setAsking(false);
    }
  };

  const follow = (fk: ForeignKeyDesign, value: string) =>
    openTable(connection.id, fk.refSchema ?? schema, fk.refTable, "data", [{ column: fk.refColumns[0], op: "eq", value }]);

  const exportAs = async (format: ExportFormat) => {
    if (!details || !data.current) return;
    const path = await saveDialog({ defaultPath: `${details.design.name}.${format}`, filters: [{ name: format.toUpperCase(), extensions: [format] }] });
    if (!path) return;
    toast.info(`Exporting ${details.design.name}…`);
    try {
      const n = await ipc.exportTable(connection.id, data.current.request(), format, path);
      toast.success(`Exported ${n.toLocaleString("en-US")} rows to ${path.split(/[\\/]/).pop()}`);
    } catch (e) {
      toast.error(`Export failed: ${errorMessage(e)}`);
    }
  };

  const importCsv = async () => {
    const path = await openDialog({ multiple: false, filters: [{ name: "CSV", extensions: ["csv", "tsv", "txt"] }] });
    if (typeof path === "string") setImportPath(path);
  };

  const loaded = status?.loaded ?? 0;
  const total = status?.total;

  return (
    <div className={s.view}>
      <div className={s.toolbar}>
        <div className={s.toggle} role="group" aria-label="View">
          <button aria-pressed={view === "data"} onClick={() => setView("data")}>
            Data
          </button>
          <button aria-pressed={view === "structure"} onClick={() => setView("structure")}>
            Structure
          </button>
        </div>

        {view === "data" && details && (
          <>
            <SearchBar query={query} onQuery={setQuery} onAsk={ask} asking={asking} />
            <FilterButton columns={details.design.columns} query={query} onQuery={setQuery} />
            <SortButton columns={details.design.columns} query={query} onQuery={setQuery} />
            <span className={s.spacer} />
            {(status?.selectedRows ?? 0) > 0 && editable && (
              <>
                {status!.selectedRows === 1 && (
                  <Button variant="ghost" onPress={() => data.current?.duplicateSelected()}>
                    <CopyIcon size={14} /> Duplicate
                  </Button>
                )}
                <Button variant="ghost" className={s.danger} onPress={() => data.current?.deleteSelected()}>
                  <TrashIcon size={14} /> Delete {status!.selectedRows > 1 ? `${status!.selectedRows} rows` : "row"}
                </Button>
              </>
            )}
            {dirty && (
              <>
                <span className={s.pending}>{status!.pending} unsaved</span>
                <Button variant="ghost" onPress={() => data.current?.discard()}>
                  Discard
                </Button>
                <Button variant="primary" onPress={() => data.current?.save()}>
                  Save <span className={s.kbd}>{kbd("S")}</span>
                </Button>
              </>
            )}
            {editable && !dirty && (
              <Button variant="primary" onPress={() => setInsert({})}>
                <PlusIcon size={14} /> Insert row
              </Button>
            )}
            <IconButton label="Reload" shortcut={kbd("R")} onPress={() => data.current?.refresh()}>
              {status?.loading ? <Spinner size={13} /> : <RefreshIcon size={15} />}
            </IconButton>
            <IconButton
              label="More"
              onPress={() => {
                const r = document.activeElement?.getBoundingClientRect();
                setMenu({
                  x: (r?.right ?? 0) - 220,
                  y: (r?.bottom ?? 0) + 4,
                  items: [
                    { label: "Export as CSV…", onSelect: () => exportAs("csv") },
                    { label: "Export as JSON…", onSelect: () => exportAs("json") },
                    "separator",
                    { label: "Import from CSV…", onSelect: importCsv, disabled: !editable },
                  ],
                });
              }}
            >
              <MoreIcon size={16} />
            </IconButton>
          </>
        )}
        {view === "data" && (
          <IconButton label={inspectorOpen ? "Hide details panel" : "Show details panel"} shortcut={kbd("I")} onPress={() => setSettings({ inspectorOpen: !inspectorOpen })}>
            <PanelIcon size={16} />
          </IconButton>
        )}
      </div>

      {view === "data" && <ActiveFilters query={query} onQuery={setQuery} />}

      <div className={s.body}>
        {loadError && (
          <div className={s.error} role="alert">
            <b>Couldn't read this table</b>
            <span className="selectable">{loadError}</span>
          </div>
        )}
        {!details && !loadError && <div className={s.center}>Loading…</div>}
        {details && (
          <div style={{ display: view === "data" ? "contents" : "none" }}>
            <TableData
              ref={data}
              connection={connection}
              details={details}
              query={query}
              active={active && view === "data"}
              onQuery={setQuery}
              onStatus={setStatus}
              onReview={setReview}
              onInsert={(prefill) => setInsert({ prefill })}
              onFollow={follow}
              onEditStructure={() => setView("structure")}
            />
          </div>
        )}
        {details && view === "structure" && (
          <StructureView
            connection={connection}
            details={details}
            snapshot={snapshot}
            onReview={setReview}
            onChanged={async (name) => {
              await refreshSchema(connection.id);
              if (name && name !== details.design.name) {
                patchTab(tab.id, { table: tableKey(connection.id, schema, name), tableName: name, title: name });
              } else await loadDetails();
            }}
          />
        )}
      </div>

      {view === "data" && status && (
        <div className={s.status}>
          <span>
            {total !== null && total !== undefined && total > loaded ? (
              <>
                Showing {fmt.format(loaded)} of {status.totalIsEstimate ? `~${compact.format(total)}` : fmt.format(total)} rows
              </>
            ) : (
              <>{fmt.format(loaded)} {loaded === 1 ? "row" : "rows"}</>
            )}
          </span>
          {status.elapsedMs !== null && <span className={s.faint}>{Math.round(status.elapsedMs)} ms</span>}
          {connection.readOnly && <span className={s.faint}>Read-only connection</span>}
          {connection.env === "production" && !connection.readOnly && <span className={s.faint}>Production: changes wait for Save</span>}
          <span className={s.spacer} />
          {developerMode && status.sql && (
            <button className={s.link} onClick={() => onOpenSql(status.sql!)} title="Open this query in the SQL editor">
              SQL
            </button>
          )}
        </div>
      )}

      {details && (
        <InsertRowSheet
          isOpen={insert !== null}
          connection={connection}
          details={details}
          prefill={insert?.prefill}
          onClose={() => setInsert(null)}
          onInserted={() => data.current?.refresh()}
        />
      )}
      <ReviewDialog request={review} kind={connection.kind} env={connection.env} onClose={() => setReview(null)} onOpenInEditor={onOpenSql} />
      <ContextMenu menu={menu} onClose={() => setMenu(null)} />
      {details && <ImportSheet path={importPath} connection={connection} details={details} onClose={() => setImportPath(null)} onImported={() => data.current?.refresh()} />}
    </div>
  );
}
