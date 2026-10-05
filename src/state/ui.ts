import { create } from "zustand";

export type SettingsSection = "general" | "ai" | "about";

interface UiState {
  /** Which settings section is open, or null when the window is closed. */
  settings: SettingsSection | null;
  openSettings(section?: SettingsSection): void;
  closeSettings(): void;
}

export const useUi = create<UiState>((set) => ({
  settings: null,
  openSettings: (section = "general") => set({ settings: section }),
  closeSettings: () => set({ settings: null }),
}));
