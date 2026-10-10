import { create } from "zustand";

export type SettingsSection = "general" | "ai" | "about";

/** Full-window tools that open over the workspace. */
export type Tool = "diagram" | "compare" | "backup" | "objects";

interface UiState {
  /** Which settings section is open, or null when the window is closed. */
  settings: SettingsSection | null;
  openSettings(section?: SettingsSection): void;
  closeSettings(): void;
  palette: boolean;
  setPalette(open: boolean): void;
  tool: Tool | null;
  openTool(tool: Tool | null): void;
}

export const useUi = create<UiState>((set) => ({
  settings: null,
  openSettings: (section = "general") => set({ settings: section }),
  closeSettings: () => set({ settings: null }),
  palette: false,
  setPalette: (palette) => set({ palette }),
  tool: null,
  openTool: (tool) => set({ tool }),
}));
