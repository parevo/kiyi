import { getVersion } from "@tauri-apps/api/app";
import { useEffect, useState } from "react";
import { Dialog, Heading, Modal, ModalOverlay } from "react-aria-components";
import { errorMessage, ipc } from "../lib/ipc";
import type { AiProvider, AiSettings, ProviderPreset, ProviderView } from "../lib/types";
import { useSettings } from "../state/settings";
import { toast } from "../state/toasts";
import { type SettingsSection, useUi } from "../state/ui";
import { CheckIcon, CloseIcon, EditIcon, PlusIcon, SparklesIcon, Spinner, TrashIcon } from "./icons";
import { Button, IconButton, Segmented, Switch } from "./ui";
import f from "./Form.module.css";
import s from "./SettingsDialog.module.css";
import { kbd } from "../lib/platform";

const SECTIONS: { id: SettingsSection; label: string }[] = [
  { id: "general", label: "General" },
  { id: "ai", label: "AI" },
  { id: "about", label: "About & updates" },
];

export function SettingsDialog() {
  const section = useUi((st) => st.settings);
  const open = useUi((st) => st.openSettings);
  const close = useUi((st) => st.closeSettings);

  return (
    <ModalOverlay isOpen={section !== null} onOpenChange={(o) => !o && close()} isDismissable className={s.overlay}>
      <Modal className={s.modal}>
        <Dialog className={s.dialog} aria-label="Settings">
          <nav className={s.nav}>
            <Heading slot="title" className={s.navTitle}>
              Settings
            </Heading>
            {SECTIONS.map((x) => (
              <button key={x.id} className={s.navItem} aria-current={section === x.id || undefined} onClick={() => open(x.id)}>
                {x.id === "ai" && <SparklesIcon size={14} />}
                {x.label}
              </button>
            ))}
          </nav>
          <div className={s.content}>
            <div className={s.close}>
              <IconButton label="Close settings" onPress={close}>
                <CloseIcon />
              </IconButton>
            </div>
            {section === "general" && <General />}
            {section === "ai" && <AiSection />}
            {section === "about" && <About />}
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

function General() {
  const settings = useSettings();
  return (
    <section className={s.section}>
      <h2 className={s.title}>General</h2>
      <div className={f.stack}>
        <Segmented
          label="Appearance"
          value={settings.theme}
          onChange={(theme) => settings.set({ theme })}
          options={[
            { value: "system", label: "Match system" },
            { value: "light", label: "Light" },
            { value: "dark", label: "Dark" },
          ]}
        />
        <div className={s.row}>
          <div>
            <div className={f.label}>Developer mode</div>
            <p className={f.help}>Shows the SQL behind every action, raw column types, and the SQL editor.</p>
          </div>
          <Switch isSelected={settings.developerMode} onChange={(developerMode) => settings.set({ developerMode })}>
            <span className={s.srOnly}>Developer mode</span>
          </Switch>
        </div>
        <div className={s.row}>
          <div>
            <div className={f.label}>Details panel</div>
            <p className={f.help}>The panel on the right of a table that shows the selected row. Toggle with {kbd("I")}.</p>
          </div>
          <Switch isSelected={settings.inspectorOpen} onChange={(inspectorOpen) => settings.set({ inspectorOpen })}>
            <span className={s.srOnly}>Details panel</span>
          </Switch>
        </div>
      </div>
    </section>
  );
}

function About() {
  const settings = useSettings();
  const [version, setVersion] = useState("");
  useEffect(() => {
    getVersion().then(setVersion, () => {});
  }, []);
  return (
    <section className={s.section}>
      <h2 className={s.title}>About & updates</h2>
      <div className={f.stack}>
        <p className={s.lede}>Kiyi {version}</p>
        <Segmented
          label="Update channel"
          value={settings.updateChannel}
          onChange={(updateChannel) => settings.set({ updateChannel })}
          options={[
            { value: "stable", label: "Stable" },
            { value: "beta", label: "Beta" },
          ]}
        />
        <p className={f.help}>Kiyi checks for updates in the background and asks before restarting. Beta gets new features first.</p>
        <div className={f.field}>
          <span className={f.label}>Something not working?</span>
          <div>
            <Button
              onPress={async () => {
                try {
                  await navigator.clipboard.writeText(await ipc.diagnostics());
                  toast.success("Diagnostics copied. Paste them into your bug report.");
                } catch (e) {
                  toast.error(errorMessage(e));
                }
              }}
            >
              Copy diagnostics
            </Button>
          </div>
          <span className={f.help}>Your Kiyi version, system and recent log, kept only on this computer. Passwords and keys are never logged.</span>
        </div>
      </div>
    </section>
  );
}

// ---- AI

function keyLabel(p: ProviderView, presets: ProviderPreset[]) {
  if (p.keySource === "keychain") return { text: "Key saved", ok: true };
  if (p.keySource === "environment") return { text: `Using ${presets.find((x) => x.id === p.preset)?.envVar ?? "environment"}`, ok: true };
  if (!p.needsKey) return { text: "No key needed", ok: true };
  return { text: "Needs an API key", ok: false };
}

function AiSection() {
  const [settings, setSettings] = useState<AiSettings | null>(null);
  const [presets, setPresets] = useState<ProviderPreset[]>([]);
  const [mode, setMode] = useState<{ kind: "list" } | { kind: "pick" } | { kind: "edit"; provider: AiProvider; preset: ProviderPreset | undefined; isNew: boolean }>({ kind: "list" });

  const refresh = () => ipc.aiSettings().then(setSettings, (e) => toast.error(errorMessage(e)));
  useEffect(() => {
    refresh();
    ipc.aiPresets().then(setPresets, () => {});
  }, []);

  const startNew = (preset: ProviderPreset) =>
    setMode({
      kind: "edit",
      isNew: true,
      preset,
      provider: { id: "", name: preset.id === "custom" ? "" : preset.name, kind: preset.kind, baseUrl: preset.baseUrl, model: preset.defaultModel, preset: preset.id },
    });

  if (mode.kind === "edit") {
    return (
      <ProviderForm
        initial={mode.provider}
        preset={mode.preset}
        isNew={mode.isNew}
        hasStoredKey={!!settings?.providers.find((p) => p.id === mode.provider.id)?.keySource}
        onCancel={() => setMode({ kind: "list" })}
        onSaved={() => {
          refresh();
          setMode({ kind: "list" });
        }}
      />
    );
  }

  const providers = settings?.providers ?? [];

  return (
    <section className={s.section}>
      <h2 className={s.title}>AI</h2>
      <p className={s.lede}>
        Ask for rows in plain words and AI turns your request into filters. Use any provider you like, including models running on your own Mac. Only the table's structure (column names and types) is sent, never your data.
      </p>

      {mode.kind === "pick" || providers.length === 0 ? (
        <>
          <h3 className={s.subtitle}>{providers.length ? "Add a provider" : "Choose a provider to get started"}</h3>
          <div className={s.presets}>
            {presets.map((p) => (
              <button key={p.id} className={s.preset} onClick={() => startNew(p)}>
                <span className={s.presetName}>
                  {p.name}
                  {p.local && <span className={s.tag}>On this Mac</span>}
                </span>
                <span className={s.presetDesc}>{p.description}</span>
              </button>
            ))}
          </div>
          {providers.length > 0 && (
            <Button variant="ghost" onPress={() => setMode({ kind: "list" })}>
              Cancel
            </Button>
          )}
        </>
      ) : (
        <>
          <div className={s.providers} role="radiogroup" aria-label="Active AI provider">
            {providers.map((p) => {
              const key = keyLabel(p, presets);
              const active = settings?.active === p.id;
              return (
                <div key={p.id} className={s.provider} data-active={active || undefined}>
                  <button
                    role="radio"
                    aria-checked={active}
                    className={s.radio}
                    onClick={async () => {
                      await ipc.setActiveAi(p.id);
                      refresh();
                    }}
                    aria-label={`Use ${p.name}`}
                  >
                    {active && <CheckIcon size={12} />}
                  </button>
                  <div className={s.providerMain}>
                    <span className={s.providerName}>
                      {p.name}
                      {active && <span className={s.tag}>In use</span>}
                    </span>
                    <span className={s.providerModel}>{p.model}</span>
                  </div>
                  <span className={key.ok ? s.keyOk : s.keyMissing}>{key.text}</span>
                  <IconButton
                    label={`Edit ${p.name}`}
                    onPress={() => setMode({ kind: "edit", isNew: false, provider: p, preset: presets.find((x) => x.id === p.preset) })}
                  >
                    <EditIcon size={14} />
                  </IconButton>
                  <IconButton
                    label={`Remove ${p.name}`}
                    onPress={async () => {
                      if (!confirm(`Remove ${p.name}? Its saved API key is removed too.`)) return;
                      await ipc.deleteAiProvider(p.id);
                      refresh();
                    }}
                  >
                    <TrashIcon size={14} />
                  </IconButton>
                </div>
              );
            })}
          </div>
          <Button onPress={() => setMode({ kind: "pick" })}>
            <PlusIcon size={14} /> Add provider
          </Button>
        </>
      )}
    </section>
  );
}

function ProviderForm({
  initial,
  preset,
  isNew,
  hasStoredKey,
  onCancel,
  onSaved,
}: {
  initial: AiProvider;
  preset: ProviderPreset | undefined;
  isNew: boolean;
  hasStoredKey: boolean;
  onCancel(): void;
  onSaved(): void;
}) {
  const [p, setP] = useState<AiProvider>(initial);
  const [key, setKey] = useState("");
  const [models, setModels] = useState<string[]>([]);
  const [test, setTest] = useState<{ state: "idle" | "busy" | "ok" | "error"; message?: string }>({ state: "idle" });
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const custom = preset?.id === "custom" || !preset;
  const showUrl = custom || preset?.local;

  const runTest = async () => {
    setTest({ state: "busy" });
    try {
      const list = await ipc.aiModels(p, key || null);
      setModels(list);
      if (!p.model && list.length) setP((cur) => ({ ...cur, model: list.includes(preset?.defaultModel ?? "") ? preset!.defaultModel : list[0] }));
      setTest({ state: "ok", message: list.length ? `Connected · ${list.length} models available` : "Connected" });
    } catch (e) {
      setTest({ state: "error", message: errorMessage(e) });
    }
  };

  // Local servers need no key, so load their models right away.
  useEffect(() => {
    if (preset?.local) runTest();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      await ipc.saveAiProvider({ ...p, name: p.name.trim() || preset?.name || "AI provider" }, key || null);
      toast.success(`${p.name || preset?.name} saved`);
      onSaved();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <section className={s.section}>
      <h2 className={s.title}>{isNew ? `Add ${preset?.name ?? "provider"}` : `Edit ${initial.name}`}</h2>
      {preset?.local && <p className={s.lede}>Make sure {preset.name} is running and has at least one model downloaded.</p>}
      <form
        className={f.stack}
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        {error && <div className={f.error} role="alert">{error}</div>}

        {(preset?.needsKey || custom) && (
          <label className={f.field}>
            <span className={f.label}>
              API key {!preset?.needsKey && <span className={f.meta}>if the server needs one</span>}
            </span>
            <input
              className={`${f.control} ${f.mono}`}
              type="password"
              value={key}
              onChange={(e) => setKey(e.target.value)}
              placeholder={hasStoredKey ? "Saved in Keychain. Type to replace it" : preset?.envVar ? `Paste your key (or set ${preset.envVar})` : "Paste your key"}
              autoFocus={isNew && preset?.needsKey}
            />
            {preset?.keyUrl && <span className={f.help}>Get a key at <span className="selectable">{preset.keyUrl.replace(/^https:\/\//, "")}</span>. It's stored in your system keychain.</span>}
          </label>
        )}

        {showUrl && (
          <label className={f.field}>
            <span className={f.label}>Server address</span>
            <input className={`${f.control} ${f.mono}`} value={p.baseUrl} onChange={(e) => setP({ ...p, baseUrl: e.target.value })} placeholder="https://example.com/v1" spellCheck={false} autoFocus={custom && isNew} />
            <span className={f.help}>The base URL of an OpenAI-compatible API, ending before /chat/completions.</span>
          </label>
        )}

        <div className={f.field}>
          <span className={f.label}>Model</span>
          <div className={f.withAction}>
            {/* A <datalist> filters by the current value, so once a model is filled in it only offers that one. */}
            {models.length ? (
              <select className={`${f.control} ${f.mono}`} value={p.model} onChange={(e) => setP({ ...p, model: e.target.value })}>
                {!models.includes(p.model) && <option value={p.model}>{p.model || "Choose a model"}</option>}
                {models.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
            ) : (
              <input className={`${f.control} ${f.mono}`} value={p.model} onChange={(e) => setP({ ...p, model: e.target.value })} placeholder="Test the connection to list models" spellCheck={false} />
            )}
            <Button onPress={runTest} isDisabled={test.state === "busy" || !p.baseUrl}>
              {test.state === "busy" && <Spinner />}
              Test connection
            </Button>
          </div>
          {test.state === "ok" && <span className={s.testOk}>{test.message}</span>}
          {test.state === "error" && <span className={f.error}>{test.message}</span>}
        </div>

        <label className={f.field}>
          <span className={f.label}>Name</span>
          <input className={f.control} value={p.name} onChange={(e) => setP({ ...p, name: e.target.value })} placeholder={preset?.name ?? "My server"} />
          <span className={f.help}>Shown in Settings, handy if you add the same service twice.</span>
        </label>

        <div className={s.formActions}>
          <Button onPress={onCancel}>Cancel</Button>
          <Button type="submit" variant="primary" isDisabled={saving || !p.model.trim() || !p.baseUrl.trim()}>
            {saving && <Spinner />}
            Save
          </Button>
        </div>
      </form>
    </section>
  );
}
