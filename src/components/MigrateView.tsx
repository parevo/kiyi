import { useEffect, useState } from "react";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { errorMessage, ipc } from "../lib/ipc";
import type {
  ColumnMapping,
  DbKind,
  MigrationCheck,
  MigrationIssue,
  MigrationPlan,
  MigrationProgress,
  MigrationReport,
  MigrationStep,
  SchemaSnapshot,
  TableDetails,
  TableMapping,
  ValueSource,
} from "../lib/types";
import { columnTypeLabel, useCatalog } from "../state/catalog";
import { useActiveConnection, useConnections } from "../state/connections";
import { useMigrations } from "../state/migrations";
import { toast } from "../state/toasts";
import { useUi } from "../state/ui";
import { AlertIcon, CheckIcon, CloseIcon, MoveIcon, PlusIcon, SparklesIcon, Spinner, TrashIcon } from "./icons";
import { PromptDialog, type PromptRequest } from "./PromptDialog";
import { Button, IconButton, Segmented } from "./ui";
import { Dialog, Heading, Modal, ModalOverlay } from "react-aria-components";
import d from "./dialog.module.css";
import s from "./MigrateView.module.css";

const fmt = new Intl.NumberFormat("en-US");
const norm = (x: string) => x.toLowerCase().replace(/[^a-z0-9]/g, "");
const schemaOf = (kind: DbKind, name: string) => (kind === "sqlite" ? null : name);
const keyOf = (schema: string | null, table: string) => (schema ? `${schema}.${table}` : table);

interface TableRef {
  schema: string | null;
  name: string;
  columns: string[];
}

/** Tables of a database, the connection's own schema first. */
function tablesOf(snapshot: SchemaSnapshot | undefined, kind: DbKind, includeViews: boolean): TableRef[] {
  const schemas = [...(snapshot?.schemas ?? [])].sort((a, b) => Number(b.name === snapshot?.defaultSchema) - Number(a.name === snapshot?.defaultSchema));
  return schemas.flatMap((sc) =>
    sc.tables.filter((t) => includeViews || t.kind === "table").map((t) => ({ schema: schemaOf(kind, sc.name), name: t.name, columns: t.columns.map((c) => c.name) })),
  );
}

/** A first mapping for a table the person paired by hand: same-named columns, the rest left empty. */
function guessColumns(source: TableRef, targetColumns: string[]): ColumnMapping[] {
  return targetColumns.map((t) => {
    const hit = source.columns.find((c) => norm(c) === norm(t));
    return { target: t, source: hit ? { type: "column", column: hit } : { type: "default" }, steps: [] };
  });
}

const STEP_LABEL: Record<MigrationStep["type"], string> = {
  trim: "Trim spaces",
  lower: "lower case",
  upper: "UPPER CASE",
  replace: "Replace text",
  split: "Take a part",
  map: "Change values",
  ifEmpty: "If empty",
};

function stepSummary(st: MigrationStep): string {
  switch (st.type) {
    case "replace":
      return `Replace “${st.find}” → “${st.with}”`;
    case "split": {
      const at = !st.separator || st.separator === " " ? "spaces" : `“${st.separator}”`;
      return `${st.part === 1 && !st.rest ? "First part" : `Part ${st.part}${st.rest ? " onward" : ""}`}, split at ${at}`;
    }
    case "map":
      return `Change ${st.pairs.length} ${st.pairs.length === 1 ? "value" : "values"}`;
    case "ifEmpty":
      return `If empty: ${st.value ?? "NULL"}`;
    default:
      return STEP_LABEL[st.type];
  }
}

function newStep(type: MigrationStep["type"]): MigrationStep {
  switch (type) {
    case "replace":
      return { type, find: "", with: "" };
    case "split":
      return { type, separator: " ", part: 1, rest: false };
    case "map":
      return { type, pairs: [{ from: "", to: "" }], otherwise: { type: "keep" } };
    case "ifEmpty":
      return { type, value: "" };
    default:
      return { type } as MigrationStep;
  }
}

/** Moves another database's data into the open one, through a plan the person can see and change. */
export function MigrateView() {
  const target = useActiveConnection();
  const connections = useConnections((st) => st.connections);
  const targetSnapshot = useConnections((st) => (target ? st.live[target.id]?.schema : undefined));
  const close = () => useUi.getState().openTool(null);
  const saved = useMigrations((st) => (target ? st.plans[target.id] : undefined));
  const others = connections.filter((c) => c.id !== target?.id);
  const [sourceId, setSourceId] = useState<string>(saved?.source ?? others[0]?.id ?? "");
  const [sourceSnapshot, setSourceSnapshot] = useState<SchemaSnapshot | null>(null);
  const [plan, setPlanState] = useState<MigrationPlan | null>(saved ?? null);
  const [selected, setSelected] = useState<string | null>(saved?.tables[0] ? keyOf(saved.tables[0].targetSchema, saved.tables[0].targetTable) : null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [check, setCheck] = useState<MigrationCheck | null>(null);
  const [aiNote, setAiNote] = useState<string | null>(null);
  const [prompt, setPrompt] = useState<PromptRequest | null>(null);
  const [askingAi, setAskingAi] = useState(false);
  const [running, setRunning] = useState<{ testRun: boolean; progress: MigrationProgress | null; report?: MigrationReport; error?: string; runId: string } | null>(null);
  const [details, setDetails] = useState<Record<string, TableDetails>>({});
  const source = connections.find((c) => c.id === sourceId);

  const setPlan = (next: MigrationPlan) => {
    setPlanState(next);
    setCheck(null);
    if (target) useMigrations.getState().keep(target.id, next);
  };

  // Connect the source in the background and read its tables.
  useEffect(() => {
    setSourceSnapshot(null);
    if (!source) return;
    let live = true;
    (async () => {
      try {
        if (useConnections.getState().live[source.id]?.status !== "connected") await ipc.connect(source.id);
        const snap = await ipc.loadSchema(source.id);
        if (live) setSourceSnapshot(snap);
      } catch (e) {
        if (live) setError(`Couldn't open ${source.name}: ${errorMessage(e)}`);
      }
    })();
    return () => {
      live = false;
    };
  }, [source]);

  if (!target) return null;
  const targetTables = tablesOf(targetSnapshot, target.kind, false);
  const ownSchema = targetSnapshot?.defaultSchema ?? null;
  const shownName = (t: TableRef) => (t.schema && t.schema !== ownSchema ? `${t.schema}.${t.name}` : t.name);
  const sourceTables = source ? tablesOf(sourceSnapshot ?? undefined, source.kind, true) : [];
  const mappingFor = (key: string) => plan?.tables.find((t) => keyOf(t.targetSchema, t.targetTable) === key) ?? null;

  const withBusy = async (label: string, f: () => Promise<void>) => {
    setBusy(label);
    setError(null);
    try {
      await f();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  const suggest = () =>
    withBusy("Suggesting…", async () => {
      if (!source) return;
      if (useConnections.getState().live[source.id]?.status !== "connected") await ipc.connect(source.id);
      const p = await ipc.migrationSuggest(source.id, target.id);
      setPlan(p);
      setAiNote(null);
      setSelected(p.tables[0] ? keyOf(p.tables[0].targetSchema, p.tables[0].targetTable) : null);
      if (p.tables.length === 0) toast.info("No tables look alike. Pick a source for each table on the left.");
    });

  const askAi = async (examples: boolean) => {
    setAskingAi(false);
    if (!plan) return;
    await withBusy("Asking the AI…", async () => {
      const r = await ipc.migrationAi(plan, examples);
      setPlan(r.plan);
      setAiNote(r.explanation);
    });
  };

  const runCheck = () =>
    withBusy("Checking…", async () => {
      if (plan) setCheck(await ipc.migrationCheck(plan));
    });

  const savePlan = async () => {
    if (!plan) return;
    const path = await saveDialog({ defaultPath: `${source?.name ?? "source"} to ${target.name}.kiyi-move.json`, filters: [{ name: "Kiyi migration plan", extensions: ["json"] }] });
    if (!path) return;
    try {
      await ipc.migrationSave({ ...plan, sourceName: source?.name ?? "", targetName: target.name }, path);
      toast.success("Plan saved");
    } catch (e) {
      toast.error(errorMessage(e));
    }
  };

  const openPlan = async () => {
    const path = await openDialog({ multiple: false, filters: [{ name: "Kiyi migration plan", extensions: ["json"] }] });
    if (typeof path !== "string") return;
    try {
      const p = await ipc.migrationOpen(path);
      // Plans name connections by id; on another computer, match them by name or use what's chosen here.
      const src = connections.find((c) => c.id === p.source) ?? connections.find((c) => c.name === p.sourceName && c.id !== target.id) ?? source;
      if (!src) throw new Error("Add a connection to the source database first.");
      setSourceId(src.id);
      setPlan({ ...p, source: src.id, target: target.id });
      setSelected(p.tables[0] ? keyOf(p.tables[0].targetSchema, p.tables[0].targetTable) : null);
      if (p.targetName && p.targetName !== target.name) toast.info(`This plan was made for ${p.targetName}; it now targets ${target.name}.`);
    } catch (e) {
      toast.error(errorMessage(e));
    }
  };

  const start = async (testRun: boolean) => {
    if (!plan) return;
    const runId = crypto.randomUUID();
    setRunning({ testRun, progress: null, runId });
    try {
      const report = await ipc.migrationRun(plan, testRun, runId, (progress) => setRunning((r) => (r ? { ...r, progress } : r)));
      setRunning((r) => (r ? { ...r, report } : r));
      if (!testRun) await useConnections.getState().refreshSchema(target.id);
    } catch (e) {
      setRunning((r) => (r ? { ...r, error: errorMessage(e) } : r));
    }
  };

  const confirmMove = () => {
    if (!plan) return;
    const dbName = target.database ?? target.name;
    const tables = plan.tables.filter((t) => t.enabled).length;
    const rows = check?.tables.reduce((n, t) => n + t.rows, 0);
    setPrompt({
      title: `Move data into ${target.name}?`,
      subtitle: `${source?.name} → ${target.name}${target.env === "production" ? " · production" : ""}`,
      help: `${rows !== undefined ? `${fmt.format(rows)} rows into ` : ""}${tables} ${tables === 1 ? "table" : "tables"}, in one transaction: if anything fails, nothing is kept.`,
      fields: target.env === "production" ? [{ label: `Type ${dbName} to continue`, mono: true }] : [],
      action: "Move data",
      run: ([typed]) => {
        if (target.env === "production" && typed?.trim() !== dbName) throw new Error(`Type ${dbName} exactly to confirm.`);
        void start(false);
      },
    });
  };

  const updateTable = (key: string, f: (t: TableMapping) => TableMapping) => {
    if (!plan) return;
    setPlan({ ...plan, tables: plan.tables.map((t) => (keyOf(t.targetSchema, t.targetTable) === key ? f(t) : t)) });
  };

  const chooseSource = (targetRef: TableRef, sourceKey: string) => {
    const base: MigrationPlan = plan ?? { source: sourceId, target: target.id, sourceName: "", targetName: "", tables: [] };
    const key = keyOf(targetRef.schema, targetRef.name);
    const rest = base.tables.filter((t) => keyOf(t.targetSchema, t.targetTable) !== key);
    const src = sourceTables.find((t) => keyOf(t.schema, t.name) === sourceKey);
    if (!src) return setPlan({ ...base, tables: rest });
    const existing = mappingFor(key);
    const mapping: TableMapping = {
      enabled: true,
      sourceSchema: src.schema,
      sourceTable: src.name,
      targetSchema: targetRef.schema,
      targetTable: targetRef.name,
      columns: guessColumns(src, targetRef.columns),
      ids: "keep",
      write: existing?.write ?? "insert",
      matchOn: existing?.matchOn ?? [],
    };
    setPlan({ ...base, source: sourceId, tables: [...rest, mapping] });
    setSelected(key);
  };

  const issuesFor = (key: string): MigrationIssue[] => {
    const name = key.split(".").pop()!;
    return check?.issues.filter((i) => i.table === name) ?? [];
  };

  const enabledCount = plan?.tables.filter((t) => t.enabled).length ?? 0;
  const current = selected ? targetTables.find((t) => keyOf(t.schema, t.name) === selected) : undefined;
  const planIssues = check?.issues.filter((i) => !i.table) ?? [];

  return (
    <div className={s.view}>
      <div className={s.toolbar}>
        <b>
          <MoveIcon size={15} /> Move data into {target.name}
        </b>
        <span className={s.muted}>from</span>
        <select
          value={sourceId}
          onChange={(e) => {
            setSourceId(e.target.value);
            setPlanState(null);
            setCheck(null);
          }}
          aria-label="Move data from"
        >
          {others.length === 0 && <option value="">Add another connection first</option>}
          {others.map((c) => (
            <option key={c.id} value={c.id}>
              {c.name}
            </option>
          ))}
        </select>
        <Button onPress={suggest} isDisabled={!source || !sourceSnapshot || busy !== null}>
          Suggest a plan
        </Button>
        <Button onPress={() => setAskingAi(true)} isDisabled={!plan || busy !== null}>
          <SparklesIcon size={14} /> Improve with AI
        </Button>
        <span className={s.spacer} />
        <Button variant="ghost" onPress={openPlan}>
          Open plan…
        </Button>
        <Button variant="ghost" onPress={savePlan} isDisabled={!plan}>
          Save plan…
        </Button>
        <IconButton label="Close" onPress={close}>
          <CloseIcon />
        </IconButton>
      </div>

      <div className={s.actionsBar}>
        <span className={s.muted}>
          {busy ? (
            <>
              <Spinner size={12} /> {busy}
            </>
          ) : plan ? (
            `${enabledCount} of ${targetTables.length} tables get data${check ? (check.ready ? " · checked, ready to move" : ` · ${check.issues.filter((i) => i.severity === "error").length} problems to fix`) : ""}`
          ) : (
            "Start with “Suggest a plan”, or pick a source for each table on the left."
          )}
        </span>
        <span className={s.spacer} />
        <Button onPress={runCheck} isDisabled={!plan || enabledCount === 0 || busy !== null}>
          <CheckIcon size={14} /> Check
        </Button>
        <Button onPress={() => start(true)} isDisabled={!check?.ready || busy !== null}>
          Test run
        </Button>
        <Button variant="primary" onPress={confirmMove} isDisabled={!check?.ready || busy !== null || target.readOnly}>
          Move data
        </Button>
      </div>

      {(error || aiNote || target.readOnly || planIssues.length > 0) && (
        <div className={s.notes}>
          {error && (
            <p className={s.error} role="alert">
              <AlertIcon size={14} /> <span className="selectable">{error}</span>
            </p>
          )}
          {target.readOnly && <p className={s.note}>{target.name} is read-only. Turn that off in its connection settings to move data into it.</p>}
          {aiNote && (
            <p className={s.note}>
              <SparklesIcon size={13} /> {aiNote}
            </p>
          )}
          {planIssues.map((i) => (
            <p key={i.message} className={i.severity === "error" ? s.error : s.note}>
              {i.message}
            </p>
          ))}
        </div>
      )}

      <div className={s.split}>
        <nav className={s.list} aria-label="Target tables">
          {targetTables.map((t) => {
            const key = keyOf(t.schema, t.name);
            const m = mappingFor(key);
            const iss = issuesFor(key);
            const errors = iss.filter((i) => i.severity === "error").length;
            const tc = check?.tables.find((x) => x.target === t.name);
            return (
              <button key={key} className={s.item} data-selected={selected === key || undefined} data-off={!m?.enabled || undefined} onClick={() => setSelected(key)}>
                <span className={s.itemTop}>
                  <span className={s.itemName}>{shownName(t)}</span>
                  {m?.enabled && check && (errors > 0 ? <span className={s.bad}>{errors}</span> : <CheckIcon size={13} className={s.good} />)}
                </span>
                <span className={s.itemSub}>{m ? `← ${m.sourceTable}${tc ? ` · ${fmt.format(tc.rows)} rows` : ""}${m.enabled ? "" : " · off"}` : "no source"}</span>
              </button>
            );
          })}
          {targetTables.length === 0 && <p className={s.muted}>{target.name} has no tables yet. Create them first, then move data into them.</p>}
        </nav>

        <div className={s.detail}>
          {!current && <p className={s.muted}>Pick a table on the left.</p>}
          {current && (
            <TableEditor
              key={selected!}
              targetRef={current}
              targetId={target.id}
              targetKind={target.kind}
              sourceTables={sourceTables}
              sourceLoading={!!source && !sourceSnapshot}
              mapping={mappingFor(selected!)}
              plan={plan}
              details={details[selected!]}
              onDetails={(d) => setDetails((x) => ({ ...x, [selected!]: d }))}
              onChooseSource={(k) => chooseSource(current, k)}
              onChange={(f) => updateTable(selected!, f)}
              check={check?.tables.find((x) => x.target === current.name) ?? null}
              issues={issuesFor(selected!)}
            />
          )}
        </div>
      </div>

      <PromptDialog request={prompt} onClose={() => setPrompt(null)} />
      {askingAi && <AiDialog source={source?.name ?? "the source"} onAsk={askAi} onClose={() => setAskingAi(false)} />}
      {running && (
        <RunPanel
          state={running}
          targetName={target.name}
          onClose={() => {
            // After a real move the old check no longer describes the target.
            if (running.report && !running.report.testRun) setCheck(null);
            setRunning(null);
          }}
        />
      )}
    </div>
  );
}

function TableEditor({
  targetRef,
  targetId,
  targetKind,
  sourceTables,
  sourceLoading,
  mapping,
  plan,
  details,
  onDetails,
  onChooseSource,
  onChange,
  check,
  issues,
}: {
  targetRef: TableRef;
  targetId: string;
  targetKind: DbKind;
  sourceTables: TableRef[];
  sourceLoading: boolean;
  mapping: TableMapping | null;
  plan: MigrationPlan | null;
  details: TableDetails | undefined;
  onDetails(d: TableDetails): void;
  onChooseSource(key: string): void;
  onChange(f: (t: TableMapping) => TableMapping): void;
  check: { rows: number; columns: string[]; preview: (string | null)[][] } | null;
  issues: MigrationIssue[];
}) {
  const drivers = useCatalog((st) => st.drivers);
  const driver = drivers.find((d) => d.kind === targetKind);
  useEffect(() => {
    if (!details) ipc.tableDetails(targetId, targetRef.schema, targetRef.name).then(onDetails, () => {});
  }, [details, targetId, targetRef, onDetails]);

  const src = mapping ? sourceTables.find((t) => t.name === mapping.sourceTable && t.schema === mapping.sourceSchema) : undefined;
  const design = details?.design;
  const keyCols = design?.columns.filter((c) => c.primaryKey).map((c) => c.name) ?? [];
  const singleNumericKey = keyCols.length === 1 && !!design?.columns.find((c) => c.name === keyCols[0])?.autoIncrement;
  const uniqueSets: string[][] = [...(keyCols.length ? [keyCols] : []), ...(design?.indexes.filter((i) => i.unique).map((i) => i.columns) ?? [])];
  const otherTables = (plan?.tables ?? []).filter((t) => t.enabled);
  const tableIssues = issues.filter((i) => !i.column);

  const setColumn = (target: string, f: (c: ColumnMapping) => ColumnMapping) =>
    onChange((t) => ({ ...t, columns: t.columns.some((c) => c.target === target) ? t.columns.map((c) => (c.target === target ? f(c) : c)) : [...t.columns, f({ target, source: { type: "default" }, steps: [] })] }));

  return (
    <div className={s.editor}>
      <div className={s.editorHead}>
        <h2>{targetRef.name}</h2>
        <label className={s.inline}>
          <span>gets its rows from</span>
          <select value={mapping ? keyOf(mapping.sourceSchema, mapping.sourceTable) : ""} onChange={(e) => onChooseSource(e.target.value)} aria-label="Source table">
            <option value="">{sourceLoading ? "Loading…" : "Nothing (leave as it is)"}</option>
            {sourceTables.map((t) => (
              <option key={keyOf(t.schema, t.name)} value={keyOf(t.schema, t.name)}>
                {keyOf(t.schema, t.name)}
              </option>
            ))}
          </select>
        </label>
        {mapping && (
          <label className={s.inline}>
            <input type="checkbox" checked={mapping.enabled} onChange={(e) => onChange((t) => ({ ...t, enabled: e.target.checked }))} /> Include in the move
          </label>
        )}
      </div>

      {mapping && (
        <>
          <div className={s.settings}>
            {singleNumericKey && (
              <Segmented<"keep" | "renumber">
                label="IDs"
                value={mapping.ids}
                onChange={(ids) => onChange((t) => ({ ...t, ids }))}
                options={[
                  { value: "keep", label: "Keep" },
                  { value: "renumber", label: "New IDs" },
                ]}
              />
            )}
            <Segmented<"insert" | "skip" | "update">
              label="When the row already exists"
              value={mapping.write}
              onChange={(write) => onChange((t) => ({ ...t, write, matchOn: t.matchOn.length ? t.matchOn : (uniqueSets[0] ?? []) }))}
              options={[
                { value: "insert", label: "Stop" },
                { value: "skip", label: "Skip" },
                { value: "update", label: "Update" },
              ]}
            />
            {mapping.write !== "insert" && (
              <label className={s.inline}>
                <span>Same row when</span>
                <select value={mapping.matchOn.join(",")} onChange={(e) => onChange((t) => ({ ...t, matchOn: e.target.value ? e.target.value.split(",") : [] }))} aria-label="Match existing rows on">
                  {uniqueSets.length === 0 && <option value="">No unique columns</option>}
                  {uniqueSets.map((set) => (
                    <option key={set.join(",")} value={set.join(",")}>
                      {set.join(" + ")} matches
                    </option>
                  ))}
                </select>
              </label>
            )}
          </div>
          <p className={s.hint}>
            {mapping.ids === "renumber"
              ? "Rows get numbers after the highest one already there; links from other moved tables follow them."
              : "Keys are copied as mapped below, or numbered by the database when left empty."}{" "}
            {mapping.write === "insert" ? "A row that clashes with an existing one stops the move, and nothing is kept." : mapping.write === "skip" ? "Rows that are already there are left as they are." : "Rows that are already there get the moved values; their keys never change."}
          </p>

          {tableIssues.map((i) => (
            <p key={i.message} className={i.severity === "error" ? s.error : s.note}>
              <AlertIcon size={13} /> {i.message}
            </p>
          ))}

          <table className={s.columns}>
            <thead>
              <tr>
                <th>Column in {targetRef.name}</th>
                <th>Gets its value from</th>
                <th>Then</th>
              </tr>
            </thead>
            <tbody>
              {(design?.columns ?? []).map((c) => {
                const m = mapping.columns.find((x) => x.target === c.name) ?? { target: c.name, source: { type: "default" } as ValueSource, steps: [] };
                const renumbered = mapping.ids === "renumber" && keyCols[0] === c.name;
                const colIssues = issues.filter((i) => i.column === c.name);
                const required = !c.nullable && c.default === null && !c.autoIncrement && !c.generated;
                return (
                  <tr key={c.name} data-problem={colIssues.some((i) => i.severity === "error") || undefined}>
                    <td>
                      <div className={s.colName}>{c.name}</div>
                      <div className={s.colType}>
                        {columnTypeLabel(c, driver)}
                        {required && <span className={s.req}>required</span>}
                        {c.enumValues.length > 0 && <span title={c.enumValues.join(", ")}> · {c.enumValues.length} allowed values</span>}
                      </div>
                    </td>
                    <td>
                      {c.generated ? (
                        <span className={s.muted}>Computed by the database</span>
                      ) : renumbered ? (
                        <span className={s.muted}>New numbers</span>
                      ) : (
                        <SourceEditor value={m.source} columns={src?.columns ?? []} tables={otherTables} onChange={(source) => setColumn(c.name, (x) => ({ ...x, source }))} />
                      )}
                      {colIssues.map((i) => (
                        <div key={i.message} className={i.severity === "error" ? s.cellError : s.cellNote}>
                          {i.message}
                        </div>
                      ))}
                    </td>
                    <td>{!c.generated && !renumbered && m.source.type !== "default" && <StepsEditor steps={m.steps} onChange={(steps) => setColumn(c.name, (x) => ({ ...x, steps }))} />}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          {!design && (
            <p className={s.muted}>
              <Spinner size={12} /> Reading {targetRef.name}…
            </p>
          )}

          {check && (
            <section className={s.preview}>
              <h3>
                Preview · {fmt.format(check.rows)} rows to move{check.preview.length ? `, first ${check.preview.length} as they'll be stored` : ""}
              </h3>
              {check.preview.length > 0 && (
                <div className={s.previewScroll}>
                  <table className={s.previewTable}>
                    <thead>
                      <tr>
                        {check.columns.map((c) => (
                          <th key={c}>{c}</th>
                        ))}
                      </tr>
                    </thead>
                    <tbody>
                      {check.preview.map((r, i) => (
                        <tr key={i}>
                          {r.map((v, j) => (
                            <td key={j} className={v === null ? s.null : undefined}>
                              {v ?? "NULL"}
                            </td>
                          ))}
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </section>
          )}
        </>
      )}
      {!mapping && <p className={s.muted}>Nothing moves into {targetRef.name}. Pick a source table above to fill it.</p>}
    </div>
  );
}

function SourceEditor({ value, columns, tables, onChange }: { value: ValueSource; columns: string[]; tables: TableMapping[]; onChange(v: ValueSource): void }) {
  const kind = value.type === "column" ? `col:${value.column}` : value.type;
  return (
    <div className={s.source}>
      <select
        value={kind}
        aria-label="Value from"
        onChange={(e) => {
          const v = e.target.value;
          if (v.startsWith("col:")) onChange({ type: "column", column: v.slice(4) });
          else if (v === "combine") onChange({ type: "combine", columns: columns.slice(0, 2), separator: " " });
          else if (v === "fixed") onChange({ type: "fixed", value: "" });
          else if (v === "reference") onChange({ type: "reference", column: columns[0] ?? "", schema: tables[0]?.sourceSchema ?? null, table: tables[0]?.sourceTable ?? "" });
          else onChange({ type: "default" });
        }}
      >
        <option value="default">Nothing (default or empty)</option>
        <optgroup label="Source column">
          {columns.map((c) => (
            <option key={c} value={`col:${c}`}>
              {c}
            </option>
          ))}
        </optgroup>
        <optgroup label="Other">
          <option value="combine">Several columns joined</option>
          <option value="fixed">The same value for every row</option>
          <option value="reference">New ID of a moved row</option>
        </optgroup>
      </select>
      {value.type === "combine" && (
        <div className={s.sub}>
          {value.columns.map((c, i) => (
            <select key={i} value={c} aria-label={`Part ${i + 1}`} onChange={(e) => onChange({ ...value, columns: value.columns.map((x, j) => (j === i ? e.target.value : x)) })}>
              {columns.map((x) => (
                <option key={x}>{x}</option>
              ))}
            </select>
          ))}
          <IconButton label="Add a column" onPress={() => onChange({ ...value, columns: [...value.columns, columns[0] ?? ""] })}>
            <PlusIcon size={13} />
          </IconButton>
          {value.columns.length > 1 && (
            <IconButton label="Remove the last column" onPress={() => onChange({ ...value, columns: value.columns.slice(0, -1) })}>
              <TrashIcon size={13} />
            </IconButton>
          )}
          <label>
            joined with <input className={s.small} value={value.separator} onChange={(e) => onChange({ ...value, separator: e.target.value })} aria-label="Separator" />
          </label>
        </div>
      )}
      {value.type === "fixed" && (
        <div className={s.sub}>
          <input value={value.value ?? ""} disabled={value.value === null} onChange={(e) => onChange({ type: "fixed", value: e.target.value })} aria-label="Fixed value" placeholder="Value" />
          <label>
            <input type="checkbox" checked={value.value === null} onChange={(e) => onChange({ type: "fixed", value: e.target.checked ? null : "" })} /> NULL
          </label>
        </div>
      )}
      {value.type === "reference" && (
        <div className={s.sub}>
          <span>the</span>
          <select
            value={keyOf(value.schema, value.table)}
            aria-label="Moved table"
            onChange={(e) => {
              const t = tables.find((x) => keyOf(x.sourceSchema, x.sourceTable) === e.target.value);
              if (t) onChange({ ...value, schema: t.sourceSchema, table: t.sourceTable });
            }}
          >
            {tables.map((t) => (
              <option key={keyOf(t.sourceSchema, t.sourceTable)} value={keyOf(t.sourceSchema, t.sourceTable)}>
                {t.sourceTable} (→ {t.targetTable})
              </option>
            ))}
          </select>
          <span>row whose key is in</span>
          <select value={value.column} aria-label="Source column with the key" onChange={(e) => onChange({ ...value, column: e.target.value })}>
            {columns.map((c) => (
              <option key={c}>{c}</option>
            ))}
          </select>
        </div>
      )}
    </div>
  );
}

function StepsEditor({ steps, onChange }: { steps: MigrationStep[]; onChange(steps: MigrationStep[]): void }) {
  const [open, setOpen] = useState<number | null>(null);
  const set = (i: number, st: MigrationStep) => onChange(steps.map((x, j) => (j === i ? st : x)));
  return (
    <div className={s.steps}>
      {steps.map((st, i) => (
        <div key={i} className={s.step}>
          <button className={s.chip} onClick={() => setOpen(open === i ? null : i)} aria-expanded={open === i}>
            {stepSummary(st)}
          </button>
          <IconButton label="Remove this step" onPress={() => onChange(steps.filter((_, j) => j !== i))}>
            <CloseIcon size={12} />
          </IconButton>
          {open === i && <StepForm step={st} onChange={(x) => set(i, x)} />}
        </div>
      ))}
      <select
        className={s.addStep}
        value=""
        aria-label="Add a step"
        onChange={(e) => {
          if (!e.target.value) return;
          onChange([...steps, newStep(e.target.value as MigrationStep["type"])]);
          if (["replace", "split", "map", "ifEmpty"].includes(e.target.value)) setOpen(steps.length);
        }}
      >
        <option value="">+ Step</option>
        {Object.entries(STEP_LABEL).map(([k, v]) => (
          <option key={k} value={k}>
            {v}
          </option>
        ))}
      </select>
    </div>
  );
}

function StepForm({ step, onChange }: { step: MigrationStep; onChange(s: MigrationStep): void }) {
  switch (step.type) {
    case "replace":
      return (
        <div className={s.stepForm}>
          <input value={step.find} placeholder="Find" aria-label="Find" onChange={(e) => onChange({ ...step, find: e.target.value })} />
          <input value={step.with} placeholder="Replace with" aria-label="Replace with" onChange={(e) => onChange({ ...step, with: e.target.value })} />
        </div>
      );
    case "split":
      return (
        <div className={s.stepForm}>
          <label>
            Split at <input className={s.small} value={step.separator} placeholder="space" onChange={(e) => onChange({ ...step, separator: e.target.value })} aria-label="Separator" />
          </label>
          <label>
            take part <input className={s.small} type="number" min={1} value={step.part} onChange={(e) => onChange({ ...step, part: Math.max(1, Number(e.target.value) || 1) })} aria-label="Part" />
          </label>
          <label>
            <input type="checkbox" checked={step.rest} onChange={(e) => onChange({ ...step, rest: e.target.checked })} /> and everything after it
          </label>
        </div>
      );
    case "ifEmpty":
      return (
        <div className={s.stepForm}>
          <input value={step.value ?? ""} disabled={step.value === null} placeholder="Value" aria-label="Value when empty" onChange={(e) => onChange({ ...step, value: e.target.value })} />
          <label>
            <input type="checkbox" checked={step.value === null} onChange={(e) => onChange({ ...step, value: e.target.checked ? null : "" })} /> NULL
          </label>
        </div>
      );
    case "map":
      return (
        <div className={s.stepForm}>
          {step.pairs.map((p, i) => (
            <div key={i} className={s.pair}>
              <input value={p.from} placeholder="When it's" aria-label="Value" onChange={(e) => onChange({ ...step, pairs: step.pairs.map((x, j) => (j === i ? { ...x, from: e.target.value } : x)) })} />
              <span>→</span>
              <input value={p.to ?? ""} placeholder="write" aria-label="Becomes" onChange={(e) => onChange({ ...step, pairs: step.pairs.map((x, j) => (j === i ? { ...x, to: e.target.value } : x)) })} />
              <IconButton label="Remove" onPress={() => onChange({ ...step, pairs: step.pairs.filter((_, j) => j !== i) })}>
                <CloseIcon size={12} />
              </IconButton>
            </div>
          ))}
          <Button variant="ghost" onPress={() => onChange({ ...step, pairs: [...step.pairs, { from: "", to: "" }] })}>
            <PlusIcon size={13} /> Add a value
          </Button>
          <label>
            Anything else:{" "}
            <select
              value={step.otherwise.type}
              aria-label="Anything else"
              onChange={(e) => onChange({ ...step, otherwise: e.target.value === "value" ? { type: "value", value: "" } : ({ type: e.target.value } as { type: "keep" } | { type: "null" }) })}
            >
              <option value="keep">stays as it is</option>
              <option value="null">becomes empty (NULL)</option>
              <option value="value">becomes…</option>
            </select>
            {step.otherwise.type === "value" && (
              <input className={s.small} value={step.otherwise.value} aria-label="Other values become" onChange={(e) => onChange({ ...step, otherwise: { type: "value", value: e.target.value } })} />
            )}
          </label>
        </div>
      );
    default:
      return null;
  }
}

function AiDialog({ source, onAsk, onClose }: { source: string; onAsk(examples: boolean): void; onClose(): void }) {
  const [examples, setExamples] = useState(false);
  return (
    <ModalOverlay isOpen onOpenChange={(o) => !o && onClose()} isDismissable className={d.overlay}>
      <Modal className={`${d.modal} ${d.small}`}>
        <Dialog className={d.dialog}>
          <div className={d.header}>
            <Heading slot="title" className={d.title}>
              Improve the plan with AI
            </Heading>
            <IconButton label="Close" onPress={onClose}>
              <CloseIcon />
            </IconButton>
          </div>
          <div className={d.body}>
            <p className={d.subtitle}>The AI sees the names and types of both databases' tables and columns, and suggests how each column should be filled. You review everything before anything moves.</p>
            <label className={s.inline}>
              <input type="checkbox" checked={examples} onChange={(e) => setExamples(e.target.checked)} /> Also show it three example rows from each table in {source}
            </label>
            <p className={d.subtitle}>Examples help it match values like status codes, but they leave this computer. Leave this off for sensitive data.</p>
          </div>
          <div className={d.footer}>
            <div className={d.footerRight}>
              <Button onPress={onClose}>Cancel</Button>
              <Button variant="primary" onPress={() => onAsk(examples)}>
                <SparklesIcon size={14} /> Ask AI
              </Button>
            </div>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

const STAGE: Record<MigrationProgress["stage"], string> = {
  preparing: "Preparing",
  reading: "Reading and converting",
  writing: "Writing",
};

function RunPanel({ state, targetName, onClose }: { state: { testRun: boolean; progress: MigrationProgress | null; report?: MigrationReport; error?: string; runId: string }; targetName: string; onClose(): void }) {
  const { progress, report, error, testRun } = state;
  const finished = !!report || !!error;
  // Reading fills the first half of the bar, writing the second.
  const share = progress && progress.total > 0 ? progress.done / progress.total : 0;
  const pct = progress?.stage === "writing" ? 50 + share * 50 : progress?.stage === "reading" ? share * 50 : 0;
  return (
    <div className={s.overlay} role="dialog" aria-label={testRun ? "Test run" : "Moving data"}>
      <div className={s.runBox}>
        <h2>{testRun ? "Test run" : `Moving data into ${targetName}`}</h2>
        {!finished && (
          <>
            <div className={s.bar}>
              <span style={{ width: `${pct}%` }} />
            </div>
            <p className={s.muted}>
              {progress ? `${STAGE[progress.stage]}${progress.table ? ` ${progress.table}` : ""} · ${fmt.format(progress.done)} of ${fmt.format(progress.total)} rows` : "Starting…"}
            </p>
            <p className={s.hint}>Everything is written in one transaction{testRun ? " and rolled back at the end" : ""}; stopping keeps nothing.</p>
            <Button onPress={() => ipc.migrationCancel(state.runId)}>Stop</Button>
          </>
        )}
        {error && (
          <>
            <p className={s.error} role="alert">
              <AlertIcon size={14} /> <span className="selectable">{error}</span>
            </p>
            <Button onPress={onClose}>Close</Button>
          </>
        )}
        {report && (
          <>
            <p className={s.ok}>
              <CheckIcon size={15} />{" "}
              {report.testRun
                ? `All ${fmt.format(report.rows)} rows were accepted by ${targetName}, then rolled back. Nothing was kept.`
                : `Moved ${fmt.format(report.rows)} rows in ${report.seconds < 1 ? "under a second" : `${report.seconds.toFixed(1)} seconds`}.`}
            </p>
            <ul className={s.report}>
              {report.tables.map((t) => (
                <li key={t.target}>
                  <span>{t.target}</span>
                  <span>{fmt.format(t.rows)} rows</span>
                </li>
              ))}
            </ul>
            <Button variant="primary" onPress={onClose}>
              Done
            </Button>
          </>
        )}
      </div>
    </div>
  );
}
