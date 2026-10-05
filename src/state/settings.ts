import { create } from "zustand";

export type Theme = "system" | "light" | "dark";

interface Settings {
  /** Shows SQL behind every action, the query editor shortcuts, and raw type names. */
  developerMode: boolean;
  theme: Theme;
  updateChannel: "stable" | "beta";
  inspectorOpen: boolean;
  set(patch: Partial<Omit<Settings, "set">>): void;
}

const KEY = "kiyi.settings";

function read(): Partial<Settings> {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? "{}");
  } catch {
    return {};
  }
}

export const useSettings = create<Settings>((set, get) => ({
  developerMode: false,
  theme: "system",
  updateChannel: "stable",
  inspectorOpen: true,
  ...read(),
  set(patch) {
    set(patch);
    const { set: _, ...rest } = { ...get() };
    try {
      localStorage.setItem(KEY, JSON.stringify(rest));
    } catch {
      /* settings just won't persist */
    }
  },
}));

export function applyTheme(theme: Theme) {
  const root = document.documentElement;
  if (theme === "system") delete root.dataset.theme;
  else root.dataset.theme = theme;
}
