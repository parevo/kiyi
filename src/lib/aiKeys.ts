import type { ProviderPreset } from "./types";

/** The provider a pasted key belongs to, by how it starts; the longest prefix wins (sk-ant- over sk-). */
export function presetForKey(presets: ProviderPreset[], key: string): ProviderPreset | undefined {
  const k = key.trim();
  let best: { len: number; preset: ProviderPreset } | undefined;
  for (const p of presets) {
    for (const prefix of p.keyPrefixes) {
      if (k.startsWith(prefix) && prefix.length > (best?.len ?? 0)) best = { len: prefix.length, preset: p };
    }
  }
  return best?.preset;
}

/** The model to start with: the preset's suggestion, the one already chosen, or the only one offered. */
export function pickModel(current: string, offered: string[], suggested: string | undefined): string {
  if (offered.includes(current)) return current;
  if (suggested && offered.includes(suggested)) return suggested;
  return offered.length === 1 ? offered[0] : "";
}
