import { type ReactNode, useEffect, useState } from "react";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { Dialog, Heading, Input, Modal, ModalOverlay, TextField } from "react-aria-components";
import { providerById, providerForHost } from "../lib/cloud";
import { connectionWarnings, type FieldId, nextMissing, parseSshCommand } from "../lib/connection";
import { GUIDE, type Mode, MODE_HELP } from "../lib/connectionGuide";
import { errorMessage, ipc } from "../lib/ipc";
import type { ConnectionConfig, DriverInfo, EnvTag, SshHost, SslMode, TestReport, TunnelConfig } from "../lib/types";
import { driverFor, useCatalog } from "../state/catalog";
import { useConnections } from "../state/connections";
import { AlertIcon, CheckIcon, CloseIcon, Spinner } from "./icons";
import { Button, Field, IconButton, Segmented, Switch } from "./ui";
import s from "./ConnectionDialog.module.css";
import ui from "./ui.module.css";

const blank: ConnectionConfig = {
  id: "",
  name: "",
  kind: "postgres",
  host: "localhost",
  port: 5432,
  user: "",
  database: null,
  // Encrypts when the server supports it, so cloud databases that require SSL work out of the box.
  sslMode: "prefer",
  env: "local",
  readOnly: false,
  driver: "postgres",
};

const ENV_OPTIONS: { value: EnvTag; label: React.ReactNode }[] = (
  [
    ["local", "Local"],
    ["staging", "Staging"],
    ["production", "Production"],
  ] as const
).map(([value, label]) => ({
  value,
  label: (
    <>
      <span className={s.envDot} style={{ background: `var(--env-${value})` }} />
      {label}
    </>
  ),
}));

type GuideId = keyof typeof GUIDE;

/**
 * Wires a field into the guidance: it reports focus, so its step can explain it, and shows a
 * warning when its value looks like it belongs somewhere else.
 */
type Guide = (id: GuideId) => { warning: string | null; onFocusChange(focused: boolean): void };

export function ConnectionDialog({
  editing,
  init,
  isOpen,
  onClose,
}: {
  /** Existing connection to edit; `null` creates a new one. */
  editing: ConnectionConfig | null;
  /** Prefill for a new connection: a pasted address or a chosen database. */
  init?: { url?: string; driver?: DriverInfo; host?: string; port?: number; preset?: string };
  isOpen: boolean;
  onClose(): void;
}) {
  const drivers = useCatalog((st) => st.drivers).filter((d) => d.kind);
  const save = useConnections((st) => st.save);
  const activate = useConnections((st) => st.activate);

  const [config, setConfig] = useState<ConnectionConfig>(blank);
  /** `null` = keep the password stored in the keychain (editing only). */
  const [password, setPassword] = useState<string | null>(null);
  /** SSH password or key passphrase; `null` = keep the stored one (editing only). */
  const [tunnelSecret, setTunnelSecret] = useState<string | null>(null);
  const [url, setUrl] = useState("");
  const [urlError, setUrlError] = useState<string | null>(null);
  const [nameTouched, setNameTouched] = useState(false);
  const [report, setReport] = useState<TestReport | null>(null);
  const [busy, setBusy] = useState<"test" | "save" | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** The hosted provider picked on the welcome screen, until the host says otherwise. */
  const [chosenProvider, setChosenProvider] = useState<string | null>(null);
  const [focused, setFocused] = useState<GuideId | null>(null);
  const [moreOpen, setMoreOpen] = useState(false);
  const provider = providerForHost(config.host) ?? providerById(chosenProvider ?? undefined);

  useEffect(() => {
    if (!isOpen) return;
    const d = init?.driver;
    const base = d && d.kind ? { ...blank, kind: d.kind, driver: d.id, port: d.defaultPort } : blank;
    const preset = providerById(init?.preset);
    setConfig(
      editing ?? {
        ...base,
        // A hosted database is never on localhost.
        host: init?.host ?? (preset ? "" : base.host),
        port: init?.port ?? preset?.port ?? base.port,
        sslMode: preset?.ssl ?? base.sslMode,
        env: preset ? "production" : base.env,
        readOnly: preset ? true : base.readOnly,
        name: init?.host ? `${d?.name ?? "Database"} on ${init.host}:${init.port}` : (preset?.name ?? base.name),
      },
    );
    setChosenProvider(preset?.id ?? null);
    setPassword(editing ? null : "");
    setTunnelSecret(editing ? null : "");
    setUrl("");
    setUrlError(null);
    setNameTouched(!!editing);
    setReport(null);
    setError(null);
    setFocused(null);
    setMoreOpen(false);
    if (!editing && init?.url) onUrl(init.url);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isOpen, editing, init]);

  const set = <K extends keyof ConnectionConfig>(key: K, value: ConnectionConfig[K]) => {
    setReport(null);
    setConfig((c) => {
      const next = { ...c, [key]: value };
      // Production defaults to read-only; leaving it re-enables writes.
      if (key === "env") next.readOnly = value === "production";
      return next;
    });
  };

  const onUrl = async (value: string) => {
    setUrl(value);
    setReport(null);
    if (!value.trim()) return setUrlError(null);
    try {
      const parsed = await ipc.parseConnectionUrl(value);
      setUrlError(null);
      // Keep what a URL can't say: the tunnel set up below.
      setConfig((c) => ({ ...parsed.config, id: c.id, tunnel: c.tunnel, name: nameTouched ? c.name : parsed.config.name }));
      if (parsed.password !== null) setPassword(parsed.password);
    } catch (e) {
      setUrlError(errorMessage(e));
    }
  };

  const effectiveName =
    config.name.trim() ||
    (config.kind === "sqlite"
      ? (config.database?.split(/[\\/]/).pop() ?? "SQLite")
      : config.database || config.user
        ? `${config.database || config.user} @ ${config.host}`
        : config.host);
  const missing = nextMissing(config);
  const valid = missing === null;
  const warnings = connectionWarnings(config);

  // The step being filled in explains the field in focus; with nothing in focus, the next one needed.
  const viaTunnel = config.tunnel?.type === "ssh" || config.tunnel?.type === "ssm";
  const active: GuideId | null = focused ?? (missing?.field === "host" && viaTunnel ? "hostViaTunnel" : (missing?.field ?? null));
  const guide: Guide = (id) => ({
    warning: warnings[(id === "hostViaTunnel" ? "host" : id) as FieldId] ?? null,
    onFocusChange: (on) => setFocused((cur) => (on ? id : cur === id ? null : cur)),
  });
  const aws = !provider || provider.id === "rds";
  const step = (n: number, title: string, ids: GuideId[], children: ReactNode) => (
    <Step n={n} title={title} explain={active && ids.includes(active) ? active : null} aws={aws}>
      {children}
    </Step>
  );

  const mode: Mode = config.tunnel?.type ?? "direct";
  const auth = config.auth ?? { method: "password" };
  // Options most connections never need stay folded away, unless this one already uses them.
  const usesMore = auth.method === "awsIam" || config.sslMode !== "prefer" || !!config.sslRootCert || !!provider;
  const showMore = moreOpen || usesMore;

  const test = async () => {
    setBusy("test");
    try {
      setReport(await ipc.testConnection(config, password, tunnelSecret));
    } catch (e) {
      setReport({ ok: false, steps: [{ label: "Couldn't test the connection", ok: false, detail: errorMessage(e) }], serverVersion: null });
    } finally {
      setBusy(null);
    }
  };

  const submit = async () => {
    setBusy("save");
    setError(null);
    try {
      const saved = await save({ ...config, name: effectiveName }, password, tunnelSecret);
      onClose();
      activate(saved.id);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  const onConfig = (next: ConnectionConfig) => {
    setReport(null);
    setConfig(next);
  };
  const onTunnel = (tunnel: TunnelConfig | null) => set("tunnel", tunnel);
  const onSecret = (v: string) => {
    setReport(null);
    setTunnelSecret(v);
  };

  const hostPort = (viaTunnel: boolean) => (
    <div className={s.hostRow}>
      <Field
        label={viaTunnel ? "Database host" : "Host"}
        mono
        value={config.host}
        placeholder={viaTunnel ? "localhost, or …rds.amazonaws.com" : "db.example.com, or a socket folder like /tmp"}
        onChange={(v) => set("host", v.trim())}
        {...guide(viaTunnel ? "hostViaTunnel" : "host")}
      />
      <Field label="Port" mono value={String(config.port || "")} onChange={(v) => set("port", Math.min(Number(v.replace(/\D/g, "")) || 0, 65535))} {...guide("port")} />
    </div>
  );
  const signIn = <SignInFields config={config} password={password} guide={guide} onConfig={onConfig} onPassword={(v) => { setReport(null); setPassword(v); }} />;
  const signInIds: GuideId[] = ["user", "password", "database"];

  return (
    <ModalOverlay isOpen={isOpen} onOpenChange={(open) => !open && onClose()} isDismissable className={s.overlay}>
      <Modal className={s.modal}>
        <Dialog className={s.dialog}>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              if (valid && !busy) submit();
            }}
          >
            <div className={s.header}>
              <Heading slot="title" className={s.title}>
                {editing ? "Edit connection" : "New connection"}
              </Heading>
              <IconButton label="Close" onPress={onClose}>
                <CloseIcon />
              </IconButton>
            </div>

            <div className={s.body}>
              {!editing && (
                <div className={s.paste}>
                  <TextField aria-label="Connection URL" value={url} onChange={onUrl} autoFocus>
                    <Input
                      className={`${ui.input} ${ui.mono}`}
                      placeholder={driverFor(config, drivers)?.urlExample ?? "connection URL"}
                      spellCheck={false}
                    />
                  </TextField>
                  <div className={s.pasteHint} data-error={urlError ? true : undefined}>
                    {urlError ?? "Have a connection URL? Paste it to fill in everything. Otherwise, follow the steps below."}
                  </div>
                </div>
              )}

              {provider && config.kind !== "sqlite" && (
                <div className={s.cloudHint}>
                  <b>{provider.name}</b>
                  <span>
                    Connection details: {provider.where}.
                    {provider.ssl === "require" && config.sslMode !== "require" && config.sslMode !== "verify-full" && config.sslMode !== "verify-ca" && (
                      <>
                        {" "}It requires SSL.{" "}
                        <button className={s.linkButton} onClick={() => set("sslMode", "require")}>
                          Turn it on
                        </button>
                      </>
                    )}
                    {provider.tip && ` ${provider.tip}`}
                  </span>
                </div>
              )}

              <label className={ui.field}>
                <span className={ui.label}>Database</span>
                <select
                  className={ui.input}
                  aria-label="Database type"
                  value={driverFor(config, drivers)?.id ?? ""}
                  onChange={(e) => {
                    const d = drivers.find((x) => x.id === e.target.value);
                    if (!d?.kind) return;
                    const kind = d.kind;
                    setConfig((c) => ({
                      ...c,
                      kind,
                      driver: d.id,
                      port: drivers.some((x) => x.defaultPort === c.port) ? d.defaultPort : c.port,
                    }));
                    setReport(null);
                  }}
                >
                  {drivers.map((d) => (
                    <option key={d.id} value={d.id}>
                      {d.name}
                    </option>
                  ))}
                </select>
              </label>

              {config.kind === "sqlite" ? (
                <SqliteFile
                  path={config.database ?? ""}
                  onChange={(path) => {
                    setReport(null);
                    setConfig((c) => ({ ...c, database: path || null, name: nameTouched || !path ? c.name : (path.split(/[\\/]/).pop() ?? c.name) }));
                  }}
                />
              ) : (
                <>
                  <div className={ui.field}>
                    <Segmented<Mode>
                      label="How do you reach the database?"
                      value={mode}
                      onChange={(m) => onTunnel(blankTunnel(m))}
                      options={[
                        { value: "direct", label: "Direct" },
                        { value: "ssh", label: "SSH" },
                        { value: "ssm", label: "AWS SSM" },
                        { value: "kubernetes", label: "Kubernetes" },
                        { value: "cloudSql", label: "Cloud SQL" },
                      ]}
                    />
                    <p className={s.tunnelHint}>{MODE_HELP[mode]}</p>
                  </div>

                  {config.tunnel?.type === "ssh" && (
                    <>
                      {step(1, "The SSH server", ["sshHost", "sshPort", "sshUser", "sshKey", "jumpHost"], <SshFields tunnel={config.tunnel} secret={tunnelSecret} guide={guide} onChange={onTunnel} onSecret={onSecret} />)}
                      {step(2, "The database, as that server sees it", ["hostViaTunnel", "port"], hostPort(true))}
                      {step(3, "Sign in to the database", signInIds, signIn)}
                    </>
                  )}
                  {config.tunnel?.type === "ssm" && (
                    <>
                      {step(1, "The EC2 instance to go through", ["ssmTarget", "ssmRegion", "ssmProfile"], <SsmFields tunnel={config.tunnel} guide={guide} onChange={onTunnel} />)}
                      {step(2, "The database, as that instance sees it", ["hostViaTunnel", "port"], hostPort(true))}
                      {step(3, "Sign in to the database", signInIds, signIn)}
                    </>
                  )}
                  {config.tunnel?.type === "kubernetes" && (
                    <>
                      {step(
                        1,
                        "Where the database runs",
                        ["k8sTarget", "k8sNamespace", "k8sContext", "port"],
                        <>
                          <KubernetesFields tunnel={config.tunnel} guide={guide} onChange={onTunnel} />
                          <Field
                            label="Database port in the cluster"
                            mono
                            value={String(config.port || "")}
                            onChange={(v) => set("port", Math.min(Number(v.replace(/\D/g, "")) || 0, 65535))}
                            {...guide("port")}
                          />
                        </>,
                      )}
                      {step(2, "Sign in to the database", signInIds, signIn)}
                    </>
                  )}
                  {config.tunnel?.type === "cloudSql" && (
                    <>
                      {step(
                        1,
                        "The Cloud SQL instance",
                        ["cloudSqlInstance"],
                        <Field
                          label="Instance connection name"
                          mono
                          value={config.tunnel.instance}
                          placeholder="my-project:europe-west1:my-db"
                          onChange={(v) => onTunnel({ type: "cloudSql", instance: v.trim() })}
                          {...guide("cloudSqlInstance")}
                        />,
                      )}
                      {step(2, "Sign in to the database", signInIds, signIn)}
                    </>
                  )}
                  {!config.tunnel && (
                    <>
                      {step(1, "Where the database is", ["host", "port"], hostPort(false))}
                      {step(2, "Sign in to the database", signInIds, signIn)}
                    </>
                  )}

                  <button type="button" className={s.moreToggle} aria-expanded={showMore} disabled={usesMore} onClick={() => setMoreOpen((o) => !o)}>
                    {showMore ? "▾" : "▸"} More options <span className={s.moreHint}>sign-in method, encryption (SSL)</span>
                  </button>
                  {showMore && (
                    <div className={s.more}>
                      <div className={s.row}>
                        <Segmented<"password" | "awsIam">
                          label="Database sign-in"
                          value={auth.method}
                          onChange={(m) => onConfig({ ...config, auth: m === "awsIam" ? { method: "awsIam", region: null, profile: null } : { method: "password" } })}
                          options={[
                            { value: "password", label: "Password" },
                            { value: "awsIam", label: "AWS IAM" },
                          ]}
                        />
                        <Segmented<SslMode>
                          label="Encryption (SSL)"
                          value={config.sslMode === "verify-ca" ? "verify-full" : config.sslMode}
                          onChange={(v) => set("sslMode", v)}
                          options={[
                            { value: "disable", label: "Off" },
                            { value: "prefer", label: "Prefer" },
                            { value: "require", label: "Require" },
                            { value: "verify-full", label: "Verify" },
                          ]}
                        />
                      </div>
                      <p className={s.tunnelHint}>
                        Prefer encrypts when the server supports it. Require refuses to connect without encryption; Verify also checks the server's certificate. Choose AWS IAM to sign in to RDS with your AWS
                        credentials instead of a password.
                      </p>
                      {config.tunnel?.type !== "cloudSql" && (config.sslMode === "verify-full" || config.sslMode === "verify-ca") && (
                        <CaCertField config={config} onChange={(cert) => set("sslRootCert", cert)} />
                      )}
                    </div>
                  )}
                </>
              )}

              <div className={s.saveAs}>
                <div className={s.row}>
                  <Field
                    label="Name"
                    value={config.name}
                    placeholder={effectiveName}
                    onChange={(v) => {
                      setNameTouched(true);
                      set("name", v);
                    }}
                  />
                  <Segmented<EnvTag> label="Environment" value={config.env} onChange={(v) => set("env", v)} options={ENV_OPTIONS} />
                </div>
                <div className={s.readOnly}>
                  <Switch isSelected={config.readOnly} onChange={(v) => set("readOnly", v)}>
                    Read-only
                  </Switch>
                  <span className={s.readOnlyHint}>
                    {config.readOnly ? "Data can be viewed but not changed." : "Data on this connection can be changed."}
                    {config.env === "production" && " Production connections start read-only."}
                  </span>
                </div>
              </div>

              {(report || busy === "test") && (
                <div className={s.steps} aria-live="polite" ref={(el) => el?.scrollIntoView({ block: "nearest" })}>
                  {busy === "test" && (
                    <div className={s.step}>
                      <Spinner />
                      <span>Trying {config.host}:{config.port}…</span>
                    </div>
                  )}
                  {report?.steps.map((st, i) => (
                    <div key={i} className={s.step}>
                      {st.ok ? <CheckIcon className={s.ok} /> : <AlertIcon className={s.fail} />}
                      <span>{st.label}</span>
                      {st.detail && <span className={`${s.stepDetail} selectable`}>{st.detail}</span>}
                      {st.detail?.includes("identity has changed") && config.tunnel?.type === "ssh" && (
                        <span className={s.stepDetail}>
                          <Button
                            onPress={async () => {
                              if (config.tunnel?.type !== "ssh") return;
                              await ipc.forgetHostKey(config.tunnel.host, config.tunnel.port);
                              if (config.tunnel.jump) await ipc.forgetHostKey(config.tunnel.jump.host, config.tunnel.jump.port || 22);
                              test();
                            }}
                          >
                            Trust the new identity
                          </Button>
                        </span>
                      )}
                    </div>
                  ))}
                </div>
              )}
            </div>

            <div className={s.footer}>
              <Button type="button" onPress={test} isDisabled={!valid || busy !== null}>
                {busy === "test" ? <Spinner /> : null}
                Test connection
              </Button>
              <div className={s.footerRight}>
                {error && <span className={s.error}>{error}</span>}
                {!error && missing && <span className={s.readOnlyHint}>{missing.message}</span>}
                <Button type="submit" variant="primary" isDisabled={!valid || busy !== null}>
                  {busy === "save" ? <Spinner /> : null}
                  {editing ? "Save" : "Save & connect"}
                </Button>
              </div>
            </div>
          </form>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

/** A numbered part of the form, explaining the field being filled in. */
function Step({ n, title, explain, aws, children }: { n: number; title: string; explain: GuideId | null; aws: boolean; children: ReactNode }) {
  const g = explain ? GUIDE[explain] : null;
  return (
    <section className={s.section} aria-label={title}>
      <h3 className={s.sectionTitle}>
        <span className={s.stepNo}>{n}</span>
        {title}
      </h3>
      {children}
      {g && (
        <p className={s.guide} aria-live="polite">
          {g.help}
          {aws && g.aws && (
            <>
              <br />
              <b>On AWS:</b> {g.aws}
            </>
          )}
        </p>
      )}
    </section>
  );
}

const blankSsh = (): TunnelConfig => ({ type: "ssh", host: "", port: 22, user: "", auth: { method: "key", path: "" } });

const blankTunnel = (m: Mode): TunnelConfig | null => {
  switch (m) {
    case "direct":
      return null;
    case "ssh":
      return blankSsh();
    case "ssm":
      return { type: "ssm", target: "", region: null, profile: null };
    case "kubernetes":
      return { type: "kubernetes", target: "", namespace: null, context: null };
    case "cloudSql":
      return { type: "cloudSql", instance: "" };
  }
};

type SshTunnel = Extract<TunnelConfig, { type: "ssh" }>;

/** The server Kiyi signs in to with SSH before reaching the database. */
function SshFields({
  tunnel,
  secret,
  guide,
  onChange,
  onSecret,
}: {
  tunnel: SshTunnel;
  secret: string | null;
  guide: Guide;
  onChange(t: TunnelConfig): void;
  onSecret(v: string): void;
}) {
  const [sshHosts, setSshHosts] = useState<SshHost[]>([]);
  useEffect(() => {
    ipc.sshConfigHosts().then(setSshHosts, () => setSshHosts([]));
  }, []);

  return (
    <>
      {sshHosts.length > 0 && (
        <label className={ui.field}>
          <span className={ui.label}>From your SSH config</span>
          <select
            className={ui.input}
            value=""
            onChange={(e) => {
              const h = sshHosts.find((x) => x.alias === e.target.value);
              if (!h) return;
              onChange({
                ...tunnel,
                host: h.host,
                port: h.port,
                user: h.user ?? tunnel.user,
                auth: h.identityFile ? { method: "key", path: h.identityFile } : tunnel.auth.method === "key" && !tunnel.auth.path ? { method: "agent" } : tunnel.auth,
                jump: h.jump,
              });
            }}
          >
            <option value="">Choose a host from ~/.ssh/config…</option>
            {sshHosts.map((h) => (
              <option key={h.alias} value={h.alias}>
                {h.alias}
                {h.alias !== h.host ? ` (${h.user ? `${h.user}@` : ""}${h.host})` : ""}
              </option>
            ))}
          </select>
        </label>
      )}
      <Field
        label="SSH host"
        mono
        value={tunnel.host}
        placeholder="3.120.10.5, or paste: ssh -i key.pem ec2-user@…"
        onChange={(v) => {
          // An ssh command or user@host fills in the whole step; keep typing room while one is being written.
          const cmd = parseSshCommand(v);
          if (cmd) {
            onChange({
              ...tunnel,
              host: cmd.host,
              user: cmd.user ?? tunnel.user,
              port: cmd.port ?? tunnel.port,
              auth: cmd.keyPath ? { method: "key", path: cmd.keyPath } : tunnel.auth,
              jump: cmd.jump ?? tunnel.jump,
            });
          } else onChange({ ...tunnel, host: /^ssh(\s|$)/.test(v) ? v : v.trim() });
        }}
        {...guide("sshHost")}
      />
      <div className={s.hostRow}>
        <Field label="SSH user" mono value={tunnel.user} placeholder="ec2-user or ubuntu" onChange={(v) => onChange({ ...tunnel, user: v.trim() })} {...guide("sshUser")} />
        <Field label="SSH port" mono value={String(tunnel.port || "")} onChange={(v) => onChange({ ...tunnel, port: Number(v.replace(/\D/g, "")) || 0 })} {...guide("sshPort")} />
      </div>
      <Segmented<"key" | "agent" | "password">
        label="Sign in with"
        value={tunnel.auth.method}
        onChange={(m) => onChange({ ...tunnel, auth: m === "key" ? { method: "key", path: "" } : { method: m } })}
        options={[
          { value: "key", label: "Key file (.pem)" },
          { value: "agent", label: "SSH agent" },
          { value: "password", label: "Password" },
        ]}
      />
      {tunnel.auth.method === "key" && (
        <>
          <div className={s.row}>
            <Field label="Private key" mono value={tunnel.auth.path} placeholder="~/Downloads/my-key.pem" onChange={(v) => onChange({ ...tunnel, auth: { method: "key", path: v } })} {...guide("sshKey")} />
            <Field label="Key passphrase" type="password" value={secret ?? ""} placeholder={secret === null ? "Saved in Keychain" : "if the key has one"} onChange={onSecret} />
          </div>
          <div className={s.fileActions}>
            <Button
              onPress={async () => {
                // Key files often have no extension (id_ed25519), so don't filter by one.
                const picked = await openDialog({ multiple: false, title: "Choose your private key (.pem)" });
                if (typeof picked === "string") onChange({ ...tunnel, auth: { method: "key", path: picked } });
              }}
            >
              Choose key file…
            </Button>
          </div>
        </>
      )}
      {tunnel.auth.method === "password" && <Field label="SSH password" type="password" value={secret ?? ""} placeholder={secret === null ? "Saved in Keychain" : ""} onChange={onSecret} />}
      {tunnel.auth.method === "agent" && <p className={s.tunnelHint}>Uses the keys loaded in your SSH agent (including 1Password and Secretive).</p>}
      <div className={s.readOnly}>
        <Switch isSelected={!!tunnel.jump} onChange={(on) => onChange({ ...tunnel, jump: on ? { host: "", port: 22, user: "" } : null })}>
          Through a jump host first
        </Switch>
        <span className={s.readOnlyHint}>Only if you normally SSH in through another server</span>
      </div>
      {tunnel.jump && (
        <>
          <div className={s.hostRow}>
            <Field label="Jump host" mono value={tunnel.jump.host} placeholder="gateway.example.com" onChange={(v) => onChange({ ...tunnel, jump: { ...tunnel.jump!, host: v.trim() } })} {...guide("jumpHost")} />
            <Field label="Port" mono value={String(tunnel.jump.port || "")} onChange={(v) => onChange({ ...tunnel, jump: { ...tunnel.jump!, port: Math.min(Number(v.replace(/\D/g, "")) || 0, 65535) } })} />
          </div>
          <Field label="Jump host user" mono value={tunnel.jump.user} placeholder={tunnel.user || "same as the SSH user"} onChange={(v) => onChange({ ...tunnel, jump: { ...tunnel.jump!, user: v.trim() } })} />
        </>
      )}
    </>
  );
}

function SsmFields({ tunnel, guide, onChange }: { tunnel: Extract<TunnelConfig, { type: "ssm" }>; guide: Guide; onChange(t: TunnelConfig): void }) {
  return (
    <>
      <Field label="EC2 instance ID" mono value={tunnel.target} placeholder="i-0123456789abcdef0" onChange={(v) => onChange({ ...tunnel, target: v.trim() })} {...guide("ssmTarget")} />
      <div className={s.row}>
        <Field label="Region" mono value={tunnel.region ?? ""} placeholder="from your AWS config" onChange={(v) => onChange({ ...tunnel, region: v.trim() || null })} {...guide("ssmRegion")} />
        <Field label="AWS profile" mono value={tunnel.profile ?? ""} placeholder="default" onChange={(v) => onChange({ ...tunnel, profile: v.trim() || null })} {...guide("ssmProfile")} />
      </div>
    </>
  );
}

function KubernetesFields({ tunnel, guide, onChange }: { tunnel: Extract<TunnelConfig, { type: "kubernetes" }>; guide: Guide; onChange(t: TunnelConfig): void }) {
  return (
    <>
      <Field label="Service or pod" mono value={tunnel.target} placeholder="postgres, svc/postgres or pod/postgres-0" onChange={(v) => onChange({ ...tunnel, target: v.trim() })} {...guide("k8sTarget")} />
      <div className={s.row}>
        <Field label="Namespace" mono value={tunnel.namespace ?? ""} placeholder="default" onChange={(v) => onChange({ ...tunnel, namespace: v.trim() || null })} {...guide("k8sNamespace")} />
        <Field label="Context" mono value={tunnel.context ?? ""} placeholder="current kubectl context" onChange={(v) => onChange({ ...tunnel, context: v.trim() || null })} {...guide("k8sContext")} />
      </div>
    </>
  );
}

/** The database user, its password (or AWS IAM details), and which database to open. */
function SignInFields({
  config,
  password,
  guide,
  onConfig,
  onPassword,
}: {
  config: ConnectionConfig;
  password: string | null;
  guide: Guide;
  onConfig(c: ConnectionConfig): void;
  onPassword(v: string): void;
}) {
  const auth = config.auth ?? { method: "password" };
  return (
    <>
      <div className={s.row}>
        <Field label="Database user" mono value={config.user} onChange={(v) => onConfig({ ...config, user: v.trim() })} {...guide("user")} />
        {auth.method === "password" && (
          <Field label="Password" type="password" value={password ?? ""} placeholder={password === null ? "Saved in Keychain" : ""} onChange={onPassword} {...guide("password")} />
        )}
      </div>
      {auth.method === "awsIam" && (
        <>
          <div className={s.row}>
            <Field label="AWS region" mono value={auth.region ?? ""} placeholder="from the RDS endpoint" onChange={(v) => onConfig({ ...config, auth: { ...auth, region: v.trim() || null } })} />
            <Field label="AWS profile" mono value={auth.profile ?? ""} placeholder="default" onChange={(v) => onConfig({ ...config, auth: { ...auth, profile: v.trim() || null } })} />
          </div>
          <p className={s.tunnelHint}>
            Signs in with a token from the AWS CLI (aws rds generate-db-auth-token), renewed automatically. The database user needs IAM sign-in enabled (the rds_iam role on PostgreSQL, AWSAuthenticationPlugin on MySQL). SSL is always on.
          </p>
        </>
      )}
      <Field
        label="Database name"
        mono
        value={config.database ?? ""}
        placeholder="optional"
        onChange={(v) => onConfig({ ...config, database: v.trim() ? v : null })}
        {...guide("database")}
      />
    </>
  );
}

/** The CA certificate to verify the server against. */
function CaCertField({ config, onChange }: { config: ConnectionConfig; onChange(cert: string | null): void }) {
  const rds = /\.rds\.amazonaws\.com\.?$/i.test(config.host.trim());
  return (
    <div className={s.tunnel}>
      <Field
        label="CA certificate"
        mono
        value={config.sslRootCert ?? ""}
        placeholder={rds ? "Amazon RDS bundle, downloaded automatically" : "system certificates"}
        onChange={(v) => onChange(v.trim() || null)}
      />
      <div className={s.fileActions}>
        <Button
          onPress={async () => {
            const picked = await openDialog({ multiple: false, title: "Choose the CA certificate (.pem or .crt)", filters: [{ name: "Certificate", extensions: ["pem", "crt", "cer"] }, { name: "All files", extensions: ["*"] }] });
            if (typeof picked === "string") onChange(picked);
          }}
        >
          Choose certificate…
        </Button>
        {config.sslRootCert && (
          <Button variant="ghost" onPress={() => onChange(null)}>
            Use {rds ? "the RDS bundle" : "system certificates"}
          </Button>
        )}
      </div>
      <p className={s.tunnelHint}>
        Verify checks the server's certificate before sending your password. Leave this empty for servers with a public certificate{rds ? "; for RDS, Kiyi fetches Amazon's bundle" : ""}. Through a tunnel, Kiyi checks the certificate chain but not the host name, since the connection goes to this computer.
      </p>
    </div>
  );
}

/** A SQLite database is a file: pick an existing one or create a new one. */
function SqliteFile({ path, onChange }: { path: string; onChange(path: string): void }) {
  const choose = async () => {
    const picked = await openDialog({ multiple: false, filters: [{ name: "SQLite database", extensions: ["db", "sqlite", "sqlite3", "db3"] }, { name: "All files", extensions: ["*"] }] });
    if (typeof picked === "string") onChange(picked);
  };
  const create = async () => {
    const picked = await saveDialog({ defaultPath: "database.db", filters: [{ name: "SQLite database", extensions: ["db"] }] });
    if (picked) onChange(picked);
  };
  return (
    <div className={s.tunnel}>
      <Field label="Database file" mono value={path} placeholder="/path/to/database.db" onChange={onChange} />
      <div className={s.fileActions}>
        <Button onPress={choose}>Choose file…</Button>
        <Button variant="ghost" onPress={create}>
          New database…
        </Button>
      </div>
      <p className={s.tunnelHint}>Kiyi opens the file directly. A new file is created if it doesn't exist yet.</p>
    </div>
  );
}
