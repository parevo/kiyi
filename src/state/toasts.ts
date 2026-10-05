import { create } from "zustand";

export interface Toast {
  id: number;
  text: string;
  tone: "success" | "error" | "info";
  action?: { label: string; run(): void };
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
    setTimeout(() => get().dismiss(id), t.tone === "error" ? 8000 : 3500);
  },
  dismiss: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
}));

export const toast = {
  success: (text: string) => useToasts.getState().push({ text, tone: "success" }),
  error: (text: string) => useToasts.getState().push({ text, tone: "error" }),
  info: (text: string) => useToasts.getState().push({ text, tone: "info" }),
};
