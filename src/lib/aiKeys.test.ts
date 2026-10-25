import { describe, expect, it } from "vitest";
import { pickModel, presetForKey } from "./aiKeys";
import type { ProviderPreset } from "./types";

const preset = (id: string, keyPrefixes: string[]): ProviderPreset => ({
  id,
  name: id,
  kind: "openAi",
  baseUrl: "",
  defaultModel: "",
  needsKey: true,
  local: false,
  keyUrl: null,
  envVar: null,
  description: "",
  keyPrefixes,
});
const presets = [preset("anthropic", ["sk-ant-"]), preset("openai", ["sk-"]), preset("openrouter", ["sk-or-"]), preset("xai", ["xai-"]), preset("groq", ["gsk_"])];

describe("presetForKey", () => {
  it("tells keys apart by how they start", () => {
    expect(presetForKey(presets, "xai-abc")?.id).toBe("xai");
    expect(presetForKey(presets, " gsk_abc")?.id).toBe("groq");
    expect(presetForKey(presets, "sk-ant-api03")?.id).toBe("anthropic");
    expect(presetForKey(presets, "sk-or-v1")?.id).toBe("openrouter");
    expect(presetForKey(presets, "sk-proj-1")?.id).toBe("openai");
    expect(presetForKey(presets, "0123abcd")).toBeUndefined();
  });
});

describe("pickModel", () => {
  it("keeps a model the provider offers and drops one it doesn't", () => {
    expect(pickModel("llama", ["llama", "qwen"], undefined)).toBe("llama");
    expect(pickModel("grok-build-0.1", ["llama", "qwen"], undefined)).toBe("");
    expect(pickModel("", ["llama", "qwen"], "qwen")).toBe("qwen");
    expect(pickModel("", ["only"], undefined)).toBe("only");
  });
});
