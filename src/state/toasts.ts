import { create } from "zustand";

export interface Toast {
  id: number;
  text: string;
  tone: "success" | "error" | "info";
  action?: { label: string; run(): void };
  /** Stays until clicked, for questions the person shouldn't miss. */
  sticky?: boolean;
}

interface ToastState {
  toasts: Toast[];
  push(t: Omit<Toast, "id">): void;
  dismiss(id: number): void;
}

let next = 1;

export const useToasts = create<ToastState>((set, get) => ({
  toasts: [],
  push(t) {
    const id = next++;
    set((s) => ({ toasts: [...s.toasts.slice(-3), { ...t, id }] }));
    if (!t.sticky) setTimeout(() => get().dismiss(id), t.tone === "error" ? 8000 : t.action ? 6000 : 3500);
  },
  dismiss: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
}));

export const toast = {
  success: (text: string, action?: Toast["action"]) => useToasts.getState().push({ text, tone: "success", action }),
  error: (text: string) => useToasts.getState().push({ text, tone: "error" }),
  info: (text: string) => useToasts.getState().push({ text, tone: "info" }),
  ask: (text: string, action: NonNullable<Toast["action"]>) => useToasts.getState().push({ text, tone: "info", action, sticky: true }),
};
