import { getVersion } from "@tauri-apps/api/app";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { Dialog, Heading, Modal, ModalOverlay } from "react-aria-components";
import { errorMessage, ipc } from "../lib/ipc";
import { reportProblem } from "../lib/report";
import { pickModel, presetForKey } from "../lib/aiKeys";
import type { AiCheck, AiProvider, AiSettings, ProviderPreset, ProviderView } from "../lib/types";
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
            <p className={f.help}>Shows the SQL behind every action and raw column types, and adds “Query with SQL” to tables.</p>
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
        <div className={s.row}>
          <div>
            <div className={f.label}>Query history</div>
            <p className={f.help}>Remember the queries you run, for History. Kept only on this computer, never in exported connections; turn it off if queries hold sensitive values.</p>
          </div>
          <Switch isSelected={settings.keepHistory} onChange={(keepHistory) => settings.set({ keepHistory })}>
            <span className={s.srOnly}>Query history</span>
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
          <div className={s.buttons}>
            <Button onPress={() => reportProblem("").catch((e) => toast.error(errorMessage(e)))}>Report a problem</Button>
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
          <span className={f.help}>
            Report a problem opens a new issue on GitHub with your Kiyi version, system and the end of the log filled in, for you to read before sending. Diagnostics are kept only on this
            computer; passwords and keys are never logged.
          </span>
        </div>
      </div>
    </section>
  );
}

// ---- AI

type Checks = Record<string, AiCheck | "busy">;

function status(p: ProviderView, check: AiCheck | "busy" | undefined) {
  if (check === "busy") return { text: "Checking…", ok: true };
  if (check?.ok) return { text: "Working", ok: true };
  if (check?.problem) return { text: "Not working", ok: false };
  if (p.needsKey && !p.keySource) return { text: "Needs an API key", ok: false };
  return { text: "", ok: true };
}

function AiSection() {
  const [settings, setSettings] = useState<AiSettings | null>(null);
  const [presets, setPresets] = useState<ProviderPreset[]>([]);
  const [checks, setChecks] = useState<Checks>({});
  const [mode, setMode] = useState<
    { kind: "list" } | { kind: "pick" } | { kind: "edit"; provider: AiProvider; preset: ProviderPreset | undefined; isNew: boolean; check?: AiCheck }
  >({ kind: "list" });

  const refresh = () =>
    ipc.aiSettings().then((next) => {
      setSettings(next);
      // Try every saved provider, so a wrong key or model shows here rather than when AI is used.
      setChecks(Object.fromEntries(next.providers.map((p) => [p.id, "busy" as const])));
      for (const p of next.providers) {
        ipc.aiCheck(p.id).then(
          (c) => setChecks((cur) => ({ ...cur, [p.id]: c })),
          (e) => setChecks((cur) => ({ ...cur, [p.id]: { ok: false, problem: errorMessage(e), keyBelongsTo: null } })),
        );
      }
    }, (e) => toast.error(errorMessage(e)));
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
  const edit = (p: ProviderView) => {
    const check = checks[p.id];
    setMode({ kind: "edit", isNew: false, provider: p, preset: presets.find((x) => x.id === p.preset), check: check === "busy" ? undefined : check });
  };

  if (mode.kind === "edit") {
    return (
      <ProviderForm
        initial={mode.provider}
        initialPreset={mode.preset}
        presets={presets}
        isNew={mode.isNew}
        check={mode.check}
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
          <p className={s.help}>Already have an API key? Pick the company it's from. Not sure? Its first letters tell: sk-ant- is Anthropic, xai- is xAI Grok, gsk_ is Groq, AIza is Google Gemini.</p>
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
              const check = checks[p.id];
              const st = status(p, check);
              const active = settings?.active === p.id;
              const problem = check !== "busy" ? check?.problem : null;
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
                  <span className={st.ok ? (check && check !== "busy" ? s.statusOk : s.keyOk) : s.keyMissing}>{st.text}</span>
                  <IconButton label={`Edit ${p.name}`} onPress={() => edit(p)}>
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
                  {problem && (
                    <div className={s.problem} role="alert">
                      <span>{problem}</span>
                      <Button onPress={() => edit(p)}>Fix it</Button>
                    </div>
                  )}
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

/** A numbered part of the provider form. */
function AiStep({ n, title, children }: { n: number; title: string; children: ReactNode }) {
  return (
    <section className={s.step} aria-label={title}>
      <h3 className={s.stepTitle}>
        <span className={s.stepNo}>{n}</span>
        {title}
      </h3>
      {children}
    </section>
  );
}

type TestState = { state: "idle" | "busy" | "ok" | "error"; message?: string };

function ProviderForm({
  initial,
  initialPreset,
  presets,
  isNew,
  check,
  hasStoredKey,
  onCancel,
  onSaved,
}: {
  initial: AiProvider;
  initialPreset: ProviderPreset | undefined;
  presets: ProviderPreset[];
  isNew: boolean;
  /** The last check of a saved provider, to explain what's wrong with it. */
  check?: AiCheck;
  hasStoredKey: boolean;
  onCancel(): void;
  onSaved(): void;
}) {
  const [p, setP] = useState<AiProvider>(initial);
  const [preset, setPreset] = useState(initialPreset);
  const [key, setKey] = useState("");
  const [models, setModels] = useState<string[]>([]);
  const [test, setTest] = useState<TestState>({ state: "idle" });
  const [saving, setSaving] = useState(false);
  // A key under the wrong company gets its own notice with a fix, so it isn't repeated here.
  const [error, setError] = useState<string | null>(check?.keyBelongsTo ? null : (check?.problem ?? null));
  /** After a failed check on Save, a second Save keeps the provider anyway. */
  const [saveAnyway, setSaveAnyway] = useState(false);
  const run = useRef(0);
  const custom = preset?.id === "custom" || !preset;
  const showUrl = custom || preset?.local;
  const needsKey = !!preset?.needsKey;
  const name = preset?.name ?? (p.name || "this server");

  // A key from another company is the most common mix-up (an xAI Grok key under Groq).
  const typedOwner = key.trim() ? presetForKey(presets, key) : undefined;
  const owner = key.trim() ? typedOwner : presets.find((x) => x.id === check?.keyBelongsTo);
  const wrongProvider = !custom && owner && owner.id !== preset?.id ? owner : undefined;

  /** Lists the models with the key as typed; the list doubles as the connection test. */
  const runTest = async (target = p): Promise<string[] | null> => {
    const id = ++run.current;
    setTest({ state: "busy" });
    try {
      const list = await ipc.aiModels(target, key || null);
      if (id !== run.current) return null;
      setModels(list);
      const model = pickModel(target.model, list, preset?.defaultModel);
      setP((cur) => ({ ...cur, model }));
      const dropped = target.model && list.length && !list.includes(target.model);
      setTest({
        state: "ok",
        message: dropped ? `Connected. ${name} doesn't offer ${target.model}, so choose one of its models below.` : list.length ? `Connected · ${list.length} models available` : "Connected",
      });
      setSaveAnyway(false);
      return list;
    } catch (e) {
      if (id !== run.current) return null;
      setTest({ state: "error", message: errorMessage(e) });
      return null;
    }
  };

  // A saved provider or a local server is tried right away; a new key once it's pasted.
  useEffect(() => {
    if (preset?.local || hasStoredKey) runTest();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(() => {
    if (key.trim().length < 20 || wrongProvider) return;
    const t = setTimeout(() => runTest(), 500);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, wrongProvider?.id]);

  const switchTo = (next: ProviderPreset) => {
    // The model stays when the new provider offers it (a Grok model set up under Groq).
    const target = { ...p, kind: next.kind, baseUrl: next.baseUrl, model: p.model || next.defaultModel, preset: next.id, name: !p.name || p.name === preset?.name ? next.name : p.name };
    setPreset(next);
    setP(target);
    setModels([]);
    setError(null);
    setTest({ state: "idle" });
    // The key stays: it's the right one, it was only filed under the wrong company.
    if (key.trim() || hasStoredKey) {
      run.current++;
      setTimeout(() => runTest(target), 0);
    }
  };

  const save = async () => {
    setError(null);
    let offered = models;
    if (test.state !== "ok" && !saveAnyway) {
      const list = await runTest();
      if (!list) {
        setSaveAnyway(true);
        setError("Kiyi couldn't confirm this works (see above). Fix it, or choose Save anyway.");
        return;
      }
      offered = list;
    }
    const model = p.model.trim();
    if (!model) return setError("Choose a model.");
    if (offered.length && !offered.includes(model)) return setError(`${name} doesn't offer ${model}. Choose one of its models.`);
    setSaving(true);
    try {
      await ipc.saveAiProvider({ ...p, model, name: p.name.trim() || preset?.name || "AI provider" }, key || null);
      toast.success(`${p.name || preset?.name} saved`);
      onSaved();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setSaving(false);
    }
  };

  let n = 0;
  const keyPlaceholder = hasStoredKey ? "Saved in Keychain. Paste a new key to replace it" : preset?.keyPrefixes.length ? `Paste your key (starts with ${preset.keyPrefixes[0]})` : "Paste your key";

  return (
    <section className={s.section}>
      <h2 className={s.title}>{isNew ? `Add ${preset?.name ?? "provider"}` : `Edit ${p.name.trim() || initial.name}`}</h2>
      {preset && !custom && <p className={s.lede}>{preset.description}.</p>}
      <form
        className={f.stack}
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        {needsKey && preset?.keyUrl && (
          <AiStep n={++n} title={hasStoredKey ? "Get a new key, if you need one" : "Get an API key"}>
            <p className={s.help}>
              Sign in at <span className="selectable">{preset.keyUrl.replace(/^https:\/\//, "")}</span>, create a key and copy it. Some providers need a payment method or credits before keys work.
            </p>
            <div className={s.buttons}>
              <Button onPress={() => ipc.openAiKeyPage(preset.id).catch((e) => toast.error(errorMessage(e)))}>Open {preset.name}'s key page</Button>
            </div>
          </AiStep>
        )}

        {preset?.local && (
          <AiStep n={++n} title={`Start ${preset.name}`}>
            <p className={s.help}>Make sure {preset.name} is running and has at least one model downloaded. Kiyi finds it at the address below.</p>
          </AiStep>
        )}

        {(needsKey || custom || showUrl) && (
          <AiStep n={++n} title={needsKey ? "Paste the key" : custom ? "Where the server is" : "Server address"}>
            {showUrl && (
              <label className={f.field}>
                <span className={f.label}>Server address</span>
                <input className={`${f.control} ${f.mono}`} value={p.baseUrl} onChange={(e) => setP({ ...p, baseUrl: e.target.value })} placeholder="https://example.com/v1" spellCheck={false} autoFocus={custom && isNew} />
                <span className={f.help}>The base URL of an OpenAI-compatible API, ending before /chat/completions.</span>
              </label>
            )}
            {(needsKey || custom) && (
              <label className={f.field}>
                <span className={f.label}>
                  API key {!needsKey && <span className={f.meta}>if the server needs one</span>}
                </span>
                <input
                  className={`${f.control} ${f.mono}`}
                  type="password"
                  value={key}
                  onChange={(e) => setKey(e.target.value)}
                  placeholder={keyPlaceholder}
                  autoFocus={isNew && needsKey}
                />
                <span className={f.help}>Kept in your system keychain and sent only to {name}.</span>
              </label>
            )}
            {wrongProvider && (
              <div className={s.problem} role="alert">
                <span>
                  This looks like a <b>{wrongProvider.name}</b> key, not a {preset?.name} one{wrongProvider.id === "xai" && preset?.id === "groq" ? ": Groq and xAI's Grok are different companies" : ""}.
                </span>
                <Button variant="primary" onPress={() => switchTo(wrongProvider)}>
                  Switch to {wrongProvider.name}
                </Button>
              </div>
            )}
          </AiStep>
        )}

        <AiStep n={++n} title="Choose a model">
          <div className={f.withAction}>
            {/* A <datalist> filters by the current value, so once a model is filled in it only offers that one. */}
            {models.length ? (
              <select className={`${f.control} ${f.mono}`} aria-label="Model" value={p.model} onChange={(e) => setP({ ...p, model: e.target.value })}>
                {!models.includes(p.model) && <option value={p.model}>{p.model || "Choose a model"}</option>}
                {models.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
            ) : (
              <input
                className={`${f.control} ${f.mono}`}
                aria-label="Model"
                value={p.model}
                onChange={(e) => setP({ ...p, model: e.target.value })}
                placeholder={needsKey ? "Paste the key above to see the models" : "Test the connection to list models"}
                spellCheck={false}
              />
            )}
            <Button onPress={() => runTest()} isDisabled={test.state === "busy" || !p.baseUrl}>
              {test.state === "busy" && <Spinner />}
              {test.state === "ok" || test.state === "error" ? "Test again" : "Test connection"}
            </Button>
          </div>
          {test.state === "busy" && <span className={f.help}>Connecting to {name}…</span>}
          {test.state === "ok" && <span className={s.testOk}>{test.message}</span>}
          {test.state === "error" && <span className={f.error}>{test.message}</span>}
          <span className={f.help}>Turning requests into filters is a small task, so a fast, inexpensive model works well: names with mini, flash, small, lite or haiku are usually those.</span>
        </AiStep>

        <label className={f.field}>
          <span className={f.label}>Name</span>
          <input className={f.control} value={p.name} onChange={(e) => setP({ ...p, name: e.target.value })} placeholder={preset?.name ?? "My server"} />
          <span className={f.help}>Shown in Settings, handy if you add the same service twice.</span>
        </label>

        {error && <div className={f.error} role="alert">{error}</div>}

        <div className={s.formActions}>
          <Button onPress={onCancel}>Cancel</Button>
          <Button type="submit" variant="primary" isDisabled={saving || test.state === "busy" || !p.baseUrl.trim()}>
            {saving && <Spinner />}
            {saveAnyway ? "Save anyway" : "Save"}
          </Button>
        </div>
      </form>
    </section>
  );
}
