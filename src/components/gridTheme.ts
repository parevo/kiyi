import type { Theme } from "@glideapps/glide-data-grid";
import { useEffect, useState } from "react";

// The grid paints on canvas, so CSS variables must be resolved to real colors.
function read(): Partial<Theme> {
  const css = getComputedStyle(document.documentElement);
  const v = (name: string) => css.getPropertyValue(name).trim();
  return {
    accentColor: v("--accent"),
    accentFg: v("--text-on-accent"),
    accentLight: v("--bg-selected"),
    textDark: v("--text"),
    textMedium: v("--text-muted"),
    textLight: v("--text-faint"),
    textHeader: v("--text-muted"),
    textHeaderSelected: v("--text-on-accent"),
    textBubble: v("--text"),
    bgIconHeader: v("--text-faint"),
    fgIconHeader: v("--bg"),
    bgCell: v("--bg"),
    bgCellMedium: v("--bg-raised"),
    bgHeader: v("--bg-raised"),
    bgHeaderHasFocus: v("--bg-active"),
    bgHeaderHovered: v("--bg-hover"),
    bgBubble: v("--bg-active"),
    bgBubbleSelected: v("--bg-selected"),
    bgSearchResult: v("--bg-selected"),
    borderColor: v("--line"),
    horizontalBorderColor: v("--line"),
    drilldownBorder: v("--line-strong"),
    linkColor: v("--accent"),
    fontFamily: v("--font-mono"),
    baseFontStyle: "12px",
    headerFontStyle: "600 11.5px",
    markerFontStyle: "10.5px",
    editorFontSize: "12px",
    cellHorizontalPadding: 10,
    cellVerticalPadding: 4,
    headerIconSize: 16,
    lineHeight: 1.4,
  };
}

export function useGridTheme() {
  const [theme, setTheme] = useState(read);
  useEffect(() => {
    const media = matchMedia("(prefers-color-scheme: dark)");
    const refresh = () => setTheme(read());
    media.addEventListener("change", refresh);
    // Canvas text measured before the webfont loads would be off; repaint once it's ready.
    document.fonts.ready.then(refresh);
    return () => media.removeEventListener("change", refresh);
  }, []);
  return theme;
}
