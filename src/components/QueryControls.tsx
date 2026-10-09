import { useEffect, useState } from "react";
import { Dialog, DialogTrigger, Popover, Button as AriaButton } from "react-aria-components";
import type { ColumnLayout } from "../lib/columnLayout";
import type { ColumnDesign, Filter, FilterOp } from "../lib/types";
import type { SavedView } from "../state/tablePrefs";
import { useSettings } from "../state/settings";
import { BookmarkIcon, CloseIcon, ColumnsIcon, FilterIcon, PlusIcon, SearchIcon, SortIcon, SparklesIcon, Spinner, TrashIcon } from "./icons";
import type { TableQuery } from "./TableData";
import { Button, IconButton } from "./ui";
import f from "./Form.module.css";
import s from "./QueryControls.module.css";
import { isMod, kbd } from "../lib/platform";

export const OP_LABEL: Record<FilterOp, string> = {
  eq: "is",
  ne: "is not",
  contains: "contains",
  notContains: "doesn't contain",
  startsWith: "starts with",
  endsWith: "ends with",
  gt: "is greater than",
  lt: "is less than",
  ge: "is at least",
  le: "is at most",
  in: "is any of",
  isNull: "is empty",
  notNull: "is not empty",
};
const NO_VALUE: FilterOp[] = ["isNull", "notNull"];

export function describeFilter(x: Filter) {
  return NO_VALUE.includes(x.op) ? `${x.column} ${OP_LABEL[x.op]}` : `${x.column} ${OP_LABEL[x.op]} ${x.value}`;
}

// ---- search / ask AI

export function SearchBar({
  query,
  onQuery,
  onAsk,
  asking,
}: {
  query: TableQuery;
  onQuery(q: TableQuery): void;
  onAsk(prompt: string): void;
  asking: boolean;
}) {
  const [text, setText] = useState(query.search ?? query.ai?.prompt ?? "");
  useEffect(() => setText(query.search ?? query.ai?.prompt ?? ""), [query.search, query.ai]);

  return (
    <form
      className={s.search}
      onSubmit={(e) => {
        e.preventDefault();
        onQuery({ ...query, search: text.trim() || null });
      }}
    >
      <SearchIcon size={15} />
      <input
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && isMod(e) && text.trim()) {
            e.preventDefault();
            onAsk(text.trim());
          }
          if (e.key === "Escape" && text) {
            setText("");
            onQuery({ ...query, search: null });
          }
        }}
        placeholder="Search, or ask AI in plain words…"
        aria-label="Search rows or ask AI"
        spellCheck={false}
      />
      <span className={s.keys}>↵ search</span>
      <button type="button" className={s.ask} onClick={() => text.trim() && onAsk(text.trim())} disabled={asking || !text.trim()} title={`Ask AI to turn this into filters (${kbd("↵")})`}>
        {asking ? <Spinner size={13} /> : <SparklesIcon size={14} />}
        Ask AI
      </button>
    </form>
  );
}

// ---- filter popover

export function FilterButton({ columns, query, onQuery }: { columns: ColumnDesign[]; query: TableQuery; onQuery(q: TableQuery): void }) {
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState<Filter[]>([]);
  const developerMode = useSettings((st) => st.developerMode);
  const [raw, setRaw] = useState("");

  const start = () => {
    setDraft(query.filters.length ? query.filters : [{ column: columns[0]?.name ?? "", op: "eq", value: "" }]);
    setRaw(query.rawWhere ?? "");
  };
  const apply = () => {
    onQuery({
      ...query,
      ai: null,
      filters: draft.filter((x) => x.column && (NO_VALUE.includes(x.op) || x.value !== "")),
      rawWhere: raw.trim() || null,
    });
    setOpen(false);
  };
  const update = (i: number, patch: Partial<Filter>) => setDraft((cur) => cur.map((x, j) => (j === i ? { ...x, ...patch } : x)));
  const count = query.filters.length + (query.rawWhere ? 1 : 0);

  return (
    <DialogTrigger
      isOpen={open}
      onOpenChange={(o) => {
        if (o) start();
        setOpen(o);
      }}
    >
      <AriaButton className={s.toolButton} data-active={count > 0 || undefined}>
        <FilterIcon size={14} /> Filter{count > 0 && <span className={s.count}>{count}</span>}
      </AriaButton>
      <Popover className={s.popover} placement="bottom start" offset={6}>
        <Dialog className={s.popDialog} aria-label="Filters">
          <form
            onSubmit={(e) => {
              e.preventDefault();
              apply();
            }}
          >
            <div className={s.popTitle}>Show rows where</div>
            {draft.length === 0 && <p className={s.popEmpty}>No filters. All rows are shown.</p>}
            {draft.map((x, i) => {
              const col = columns.find((c) => c.name === x.column);
              return (
                <div key={i} className={s.rule}>
                  <span className={s.join}>{i === 0 ? "Where" : "and"}</span>
                  <select className={f.control} value={x.column} onChange={(e) => update(i, { column: e.target.value })} aria-label="Column">
                    {columns.map((c) => (
                      <option key={c.name}>{c.name}</option>
                    ))}
                  </select>
                  <select className={f.control} value={x.op} onChange={(e) => update(i, { op: e.target.value as FilterOp })} aria-label="Condition">
                    {(Object.keys(OP_LABEL) as FilterOp[]).map((op) => (
                      <option key={op} value={op}>
                        {OP_LABEL[op]}
                      </option>
                    ))}
                  </select>
                  {NO_VALUE.includes(x.op) ? (
                    <span />
                  ) : col?.enumValues.length && (x.op === "eq" || x.op === "ne") ? (
                    <select className={f.control} value={x.value} onChange={(e) => update(i, { value: e.target.value })} aria-label="Value">
                      <option value="">Choose…</option>
                      {col.enumValues.map((v) => (
                        <option key={v}>{v}</option>
                      ))}
                    </select>
                  ) : (
                    <input
                      className={f.control}
                      value={x.value}
                      onChange={(e) => update(i, { value: e.target.value })}
                      placeholder={x.op === "in" ? "a, b, c" : "Value"}
                      autoFocus={i === draft.length - 1}
                      aria-label="Value"
                    />
                  )}
                  <IconButton label="Remove filter" onPress={() => setDraft((cur) => cur.filter((_, j) => j !== i))}>
                    <TrashIcon size={14} />
                  </IconButton>
                </div>
              );
            })}
            {developerMode && (
              <label className={s.rawRow}>
                <span className={s.join}>and</span>
                <input className={`${f.control} ${f.mono}`} value={raw} onChange={(e) => setRaw(e.target.value)} placeholder="SQL condition, e.g. total > 100" spellCheck={false} />
              </label>
            )}
            <div className={s.popFooter}>
              <Button variant="ghost" onPress={() => setDraft((cur) => [...cur, { column: columns[0]?.name ?? "", op: "eq", value: "" }])}>
                <PlusIcon size={14} /> Add filter
              </Button>
              <span className={s.spacer} />
              <Button
                variant="ghost"
                onPress={() => {
                  onQuery({ ...query, ai: null, filters: [], rawWhere: null });
                  setOpen(false);
                }}
              >
                Clear
              </Button>
              <Button type="submit" variant="primary">
                Apply
              </Button>
            </div>
          </form>
        </Dialog>
      </Popover>
    </DialogTrigger>
  );
}

// ---- sort popover

export function SortButton({ columns, query, onQuery }: { columns: ColumnDesign[]; query: TableQuery; onQuery(q: TableQuery): void }) {
  const [open, setOpen] = useState(false);
  const sort = query.sort;
  return (
    <DialogTrigger isOpen={open} onOpenChange={setOpen}>
      <AriaButton className={s.toolButton} data-active={sort ? true : undefined}>
        <SortIcon size={14} /> {sort ? `Sorted by ${sort.column}` : "Sort"}
      </AriaButton>
      <Popover className={s.popover} placement="bottom start" offset={6}>
        <Dialog className={s.popDialog} aria-label="Sort">
          <div className={s.popTitle}>Sort rows by</div>
          <div className={s.sortRow}>
            <select
              className={f.control}
              value={sort?.column ?? ""}
              onChange={(e) => onQuery({ ...query, sort: e.target.value ? { column: e.target.value, descending: sort?.descending ?? false } : null })}
              aria-label="Sort column"
            >
              <option value="">Default order</option>
              {columns.map((c) => (
                <option key={c.name}>{c.name}</option>
              ))}
            </select>
            <select
              className={f.control}
              value={sort?.descending ? "desc" : "asc"}
              disabled={!sort}
              onChange={(e) => sort && onQuery({ ...query, sort: { ...sort, descending: e.target.value === "desc" } })}
              aria-label="Direction"
            >
              <option value="asc">Ascending (A→Z, 1→9)</option>
              <option value="desc">Descending (Z→A, 9→1)</option>
            </select>
          </div>
          <p className={s.popHint}>Tip: click a column header to sort by it.</p>
        </Dialog>
      </Popover>
    </DialogTrigger>
  );
}

/** Which columns show, in what order. Dragging headers also reorders; right-click a header to freeze. */
export function ColumnsButton({ columns, layout, onLayout }: { columns: ColumnDesign[]; layout: ColumnLayout; onLayout(l: ColumnLayout): void }) {
  const hidden = new Set(layout.hidden);
  const changed = layout.hidden.length > 0 || layout.order.length > 0 || layout.frozen > 0;
  return (
    <DialogTrigger>
      <AriaButton className={s.toolButton} data-active={layout.hidden.length ? true : undefined}>
        <ColumnsIcon size={14} /> {layout.hidden.length ? `${columns.length - layout.hidden.filter((h) => columns.some((c) => c.name === h)).length} of ${columns.length} columns` : "Columns"}
      </AriaButton>
      <Popover className={s.popover} placement="bottom start" offset={6}>
        <Dialog className={s.popDialog} aria-label="Columns">
          <div className={s.popTitle}>Show columns</div>
          <div className={s.columnList}>
            {columns.map((c) => (
              <label key={c.name} className={s.columnItem}>
                <input
                  type="checkbox"
                  checked={!hidden.has(c.name)}
                  // Keep at least one column visible.
                  disabled={!hidden.has(c.name) && columns.length - hidden.size <= 1}
                  onChange={(e) => onLayout({ ...layout, hidden: e.target.checked ? layout.hidden.filter((h) => h !== c.name) : [...layout.hidden, c.name] })}
                />
                <span>{c.name}</span>
              </label>
            ))}
          </div>
          <div className={s.popActions}>
            <Button variant="ghost" onPress={() => onLayout({ ...layout, hidden: [] })} isDisabled={!layout.hidden.length}>
              Show all
            </Button>
            <Button variant="ghost" onPress={() => onLayout({ order: [], hidden: [], frozen: 0 })} isDisabled={!changed}>
              Reset
            </Button>
          </div>
          <p className={s.popHint}>Drag a column header to move it. Right-click a header to freeze columns on the left.</p>
        </Dialog>
      </Popover>
    </DialogTrigger>
  );
}

/** Saved combinations of filters, sort and columns for this table. */
export function ViewsButton({
  views,
  onApply,
  onSave,
  onDelete,
}: {
  views: SavedView[];
  onApply(v: SavedView): void;
  onSave(name: string): void;
  onDelete(v: SavedView): void;
}) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  return (
    <DialogTrigger isOpen={open} onOpenChange={setOpen}>
      <AriaButton className={s.toolButton} aria-label="Saved views">
        <BookmarkIcon size={14} /> Views{views.length ? ` (${views.length})` : ""}
      </AriaButton>
      <Popover className={s.popover} placement="bottom start" offset={6}>
        <Dialog className={s.popDialog} aria-label="Saved views">
          <div className={s.popTitle}>Saved views</div>
          {views.length === 0 && <p className={s.popHint}>Save the current filters, sort and columns to come back to them in one click.</p>}
          {views.map((v) => (
            <div key={v.id} className={s.viewRow}>
              <button
                className={s.viewName}
                onClick={() => {
                  onApply(v);
                  setOpen(false);
                }}
              >
                {v.name}
              </button>
              <IconButton label={`Delete ${v.name}`} onPress={() => onDelete(v)}>
                <TrashIcon size={13} />
              </IconButton>
            </div>
          ))}
          <form
            className={s.sortRow}
            onSubmit={(e) => {
              e.preventDefault();
              if (!name.trim()) return;
              onSave(name.trim());
              setName("");
            }}
          >
            <input className={f.control} value={name} onChange={(e) => setName(e.target.value)} placeholder="Name this view, e.g. Paid this month" aria-label="View name" />
            <Button type="submit" isDisabled={!name.trim()}>
              Save
            </Button>
          </form>
        </Dialog>
      </Popover>
    </DialogTrigger>
  );
}

// ---- active filter chips

export function ActiveFilters({ query, onQuery }: { query: TableQuery; onQuery(q: TableQuery): void }) {
  const developerMode = useSettings((st) => st.developerMode);
  const any = query.filters.length || query.rawWhere || query.search || query.sort || query.ai;
  if (!any) return null;
  return (
    <div className={s.chips}>
      {query.ai && (
        <span className={`${s.chip} ${s.aiChip}`} title={query.ai.prompt}>
          <SparklesIcon size={13} /> {query.ai.explanation}
        </span>
      )}
      {query.search && (
        <span className={s.chip}>
          Matches “{query.search}”
          <button aria-label="Remove search" onClick={() => onQuery({ ...query, search: null })}>
            <CloseIcon size={12} />
          </button>
        </span>
      )}
      {query.filters.map((x, i) => (
        <span key={i} className={s.chip}>
          {describeFilter(x)}
          <button aria-label="Remove filter" onClick={() => onQuery({ ...query, ai: null, filters: query.filters.filter((_, j) => j !== i) })}>
            <CloseIcon size={12} />
          </button>
        </span>
      ))}
      {query.rawWhere && (
        <span className={s.chip} title={query.rawWhere}>
          {developerMode ? query.rawWhere : "Custom condition"}
          <button aria-label="Remove condition" onClick={() => onQuery({ ...query, ai: null, rawWhere: null })}>
            <CloseIcon size={12} />
          </button>
        </span>
      )}
      {query.sort && (
        <span className={s.chip}>
          Sorted by {query.sort.column} {query.sort.descending ? "↓" : "↑"}
          <button aria-label="Remove sort" onClick={() => onQuery({ ...query, sort: null })}>
            <CloseIcon size={12} />
          </button>
        </span>
      )}
      <button className={s.clearAll} onClick={() => onQuery({ filters: [], rawWhere: null, search: null, sort: null, ai: null })}>
        Clear all
      </button>
    </div>
  );
}
