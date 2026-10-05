import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { EditorView } from "@codemirror/view";
import { tags as t } from "@lezer/highlight";

// Colors come from CSS variables, so the editor follows the app theme without reconfiguring.
export const editorTheme = EditorView.theme({
  "&": {
    height: "100%",
    color: "var(--syn-identifier)",
    backgroundColor: "var(--bg)",
    fontSize: "13px",
  },
  ".cm-scroller": {
    fontFamily: "var(--font-mono)",
    lineHeight: "1.65",
  },
  ".cm-content": { padding: "10px 0", caretColor: "var(--accent)" },
  ".cm-line": { padding: "0 16px 0 8px" },
  "&.cm-focused": { outline: "none" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--accent)", borderLeftWidth: "2px" },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": {
    backgroundColor: "color-mix(in srgb, var(--accent) 24%, transparent) !important",
  },
  ".cm-activeLine": { backgroundColor: "color-mix(in srgb, var(--text) 3%, transparent)" },
  ".cm-gutters": {
    backgroundColor: "var(--bg)",
    color: "var(--text-faint)",
    border: "none",
    paddingLeft: "6px",
  },
  ".cm-activeLineGutter": { backgroundColor: "transparent", color: "var(--text-muted)" },
  ".cm-lineNumbers .cm-gutterElement": { minWidth: "28px", fontSize: "11.5px" },
  ".cm-matchingBracket": {
    backgroundColor: "color-mix(in srgb, var(--accent) 18%, transparent)",
    outline: "none",
  },
  ".cm-searchMatch": { backgroundColor: "color-mix(in srgb, var(--sand) 30%, transparent)" },
  ".cm-tooltip": {
    border: "none",
    borderRadius: "var(--radius-md)",
    backgroundColor: "var(--bg-raised)",
    boxShadow: "var(--shadow-pop)",
    overflow: "hidden",
  },
  ".cm-tooltip-autocomplete > ul": { fontFamily: "var(--font-mono)", fontSize: "12px", maxHeight: "260px" },
  ".cm-tooltip-autocomplete > ul > li": { padding: "3px 10px 3px 6px !important", lineHeight: "1.5" },
  ".cm-tooltip-autocomplete > ul > li[aria-selected]": { backgroundColor: "var(--bg-selected)", color: "var(--text)" },
  ".cm-completionDetail": { color: "var(--text-faint)", fontStyle: "normal", marginLeft: "12px" },
  ".cm-completionMatchedText": { textDecoration: "none", color: "var(--accent)" },
  ".cm-completionIcon": { opacity: "0.55", width: "1.2em" },
  ".cm-panels": { backgroundColor: "var(--bg-raised)", color: "var(--text)", borderColor: "var(--line)" },
  ".cm-panel input, .cm-panel button": { fontFamily: "var(--font-ui)" },
  ".cm-error-mark": {
    textDecoration: "underline wavy var(--danger)",
    textUnderlineOffset: "3px",
    backgroundColor: "color-mix(in srgb, var(--danger) 12%, transparent)",
  },
  ".cm-run-flash": {
    backgroundColor: "color-mix(in srgb, var(--accent) 12%, transparent)",
    transition: "background-color 400ms",
  },
});

export const highlight = syntaxHighlighting(
  HighlightStyle.define([
    { tag: [t.keyword, t.operatorKeyword, t.modifier], color: "var(--syn-keyword)" },
    { tag: [t.string, t.special(t.string)], color: "var(--syn-string)" },
    { tag: [t.number, t.bool, t.null], color: "var(--syn-number)" },
    { tag: [t.lineComment, t.blockComment], color: "var(--syn-comment)", fontStyle: "italic" },
    { tag: [t.operator, t.punctuation, t.separator, t.bracket], color: "var(--syn-operator)" },
    { tag: [t.typeName, t.standard(t.name)], color: "var(--syn-type)" },
    { tag: [t.special(t.name), t.quote], color: "var(--syn-type)" },
    { tag: [t.name, t.propertyName], color: "var(--syn-identifier)" },
  ]),
);
