import { autocompletion, closeBrackets, closeBracketsKeymap, completionKeymap } from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { bracketMatching, indentOnInput } from "@codemirror/language";
import { MySQL, PostgreSQL, sql, type SQLNamespace } from "@codemirror/lang-sql";
import { highlightSelectionMatches, searchKeymap } from "@codemirror/search";
import { Compartment, EditorState, StateEffect, StateField } from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  drawSelection,
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
  placeholder,
} from "@codemirror/view";
import { useEffect, useRef } from "react";
import { statementAt } from "../lib/sql";
import type { DbKind, SchemaSnapshot } from "../lib/types";
import { editorTheme, highlight } from "./editorTheme";

export interface RunRequest {
  sql: string;
  /** Offset of `sql` in the document, for mapping server error positions back. */
  offset: number;
}

interface Props {
  value: string;
  kind: DbKind;
  schema: SchemaSnapshot | undefined;
  /** Absolute document offset of a server-reported error, or null. */
  errorAt: number | null;
  onChange(value: string): void;
  onRun(req: RunRequest): void;
  onCancel(): void;
}

const setError = StateEffect.define<number | null>();
const errorMark = Decoration.mark({ class: "cm-error-mark" });
const errorField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(marks, tr) {
    marks = marks.map(tr.changes);
    for (const e of tr.effects) {
      if (!e.is(setError)) continue;
      if (e.value === null) return Decoration.none;
      const doc = tr.state.doc;
      const from = Math.min(e.value, doc.length);
      const word = /^[\w$."`]+/.exec(doc.sliceString(from, Math.min(doc.length, from + 64)));
      const to = Math.min(doc.length, from + (word ? word[0].length : 1));
      return from < to ? Decoration.set([errorMark.range(from, to)]) : Decoration.none;
    }
    // Editing clears the stale marker.
    return tr.docChanged ? Decoration.none : marks;
  },
  provide: (f) => EditorView.decorations.from(f),
});

const flash = StateEffect.define<{ from: number; to: number } | null>();
const flashField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(marks, tr) {
    for (const e of tr.effects) {
      if (e.is(flash)) {
        return e.value && e.value.from < e.value.to
          ? Decoration.set([Decoration.mark({ class: "cm-run-flash" }).range(e.value.from, e.value.to)])
          : Decoration.none;
      }
    }
    return marks.map(tr.changes);
  },
  provide: (f) => EditorView.decorations.from(f),
});

function completionSchema(schema: SchemaSnapshot | undefined): SQLNamespace {
  const ns: { [name: string]: SQLNamespace } = {};
  for (const sc of schema?.schemas ?? []) {
    const tables: { [name: string]: SQLNamespace } = {};
    for (const t of sc.tables) {
      tables[t.name] = t.columns.map((c) => ({ label: c.name, type: "property", detail: c.dataType }));
    }
    ns[sc.name] = tables;
  }
  return ns;
}

function language(kind: DbKind, schema: SchemaSnapshot | undefined) {
  return sql({
    dialect: kind === "mysql" ? MySQL : PostgreSQL,
    schema: completionSchema(schema),
    defaultSchema: schema?.defaultSchema ?? undefined,
    upperCaseKeywords: true,
  });
}

export function QueryEditor({ value, kind, schema, errorAt, onChange, onRun, onCancel }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const lang = useRef(new Compartment());
  // Latest callbacks without rebuilding the editor.
  const props = useRef({ onChange, onRun, onCancel, kind });
  props.current = { onChange, onRun, onCancel, kind };

  useEffect(() => {
    const runAt = (v: EditorView, all: boolean) => {
      const { state } = v;
      const sel = state.selection.main;
      let req: RunRequest | null;
      if (!sel.empty) req = { sql: state.sliceDoc(sel.from, sel.to), offset: sel.from };
      else if (all) req = { sql: state.doc.toString(), offset: 0 };
      else {
        const st = statementAt(state.doc.toString(), sel.head, props.current.kind === "mysql");
        req = st && { sql: st.text, offset: st.from };
      }
      if (!req || !req.sql.trim()) return true;
      v.dispatch({ effects: flash.of({ from: req.offset, to: req.offset + req.sql.length }) });
      setTimeout(() => view.current?.dispatch({ effects: flash.of(null) }), 350);
      props.current.onRun(req);
      return true;
    };

    const v = new EditorView({
      parent: host.current!,
      state: EditorState.create({
        doc: value,
        extensions: [
          lineNumbers(),
          highlightActiveLineGutter(),
          history(),
          drawSelection(),
          indentOnInput(),
          bracketMatching(),
          closeBrackets(),
          autocompletion({ activateOnTyping: true, icons: false }),
          highlightActiveLine(),
          highlightSelectionMatches(),
          placeholder("SELECT …   ⌘↵ runs the statement under the cursor"),
          lang.current.of(language(kind, schema)),
          errorField,
          flashField,
          editorTheme,
          highlight,
          EditorView.contentAttributes.of({ spellcheck: "false", autocorrect: "off", autocapitalize: "off" }),
          keymap.of([
            { key: "Mod-Enter", run: (v) => runAt(v, false), preventDefault: true },
            { key: "Shift-Mod-Enter", run: (v) => runAt(v, true), preventDefault: true },
            { key: "Mod-.", run: () => (props.current.onCancel(), true), preventDefault: true },
            ...closeBracketsKeymap,
            ...completionKeymap,
            ...searchKeymap,
            ...historyKeymap,
            indentWithTab,
            ...defaultKeymap,
          ]),
          EditorView.updateListener.of((u) => {
            if (u.docChanged) props.current.onChange(u.state.doc.toString());
          }),
        ],
      }),
    });
    view.current = v;
    v.focus();
    return () => v.destroy();
    // The editor is created once per mount; prop changes are applied below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // External value changes (e.g. switching tabs reuses the component via `key`, so this is rare).
  useEffect(() => {
    const v = view.current;
    if (v && v.state.doc.toString() !== value) {
      v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: value } });
    }
  }, [value]);

  useEffect(() => {
    view.current?.dispatch({ effects: lang.current.reconfigure(language(kind, schema)) });
  }, [kind, schema]);

  useEffect(() => {
    view.current?.dispatch({ effects: setError.of(errorAt) });
  }, [errorAt]);

  return <div ref={host} style={{ height: "100%", overflow: "hidden" }} />;
}
