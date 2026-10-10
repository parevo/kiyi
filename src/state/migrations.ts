import { create } from "zustand";
import type { MigrationPlan } from "../lib/types";

/** The plan being worked on for each target, so closing the window doesn't lose it (until the app quits; save it to a file to keep it). */
export const useMigrations = create<{ plans: Record<string, MigrationPlan>; keep(targetId: string, plan: MigrationPlan): void }>((set) => ({
  plans: {},
  keep: (targetId, plan) => set((st) => ({ plans: { ...st.plans, [targetId]: plan } })),
}));
