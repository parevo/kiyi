import { useEffect, useState } from "react";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { Dialog, Heading, Input, Modal, ModalOverlay, TextField } from "react-aria-components";
import { missingField } from "../lib/connection";
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

export function ConnectionDialog({
  editing,
  init,
  isOpen,
  onClose,
}: {
  /** Existing connection to edit; `null` creates a new one. */
  editing: ConnectionConfig | null;
  /** Prefill for a new connection: a pasted address or a chosen database. */
  init?: { url?: string; driver?: DriverInfo; host?: string; port?: number };
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

  useEffect(() => {
    if (!isOpen) return;
    const d = init?.driver;
    const base = d && d.kind ? { ...blank, kind: d.kind, driver: d.id, port: d.defaultPort } : blank;
    setConfig(editing ?? { ...base, host: init?.host ?? base.host, port: init?.port ?? base.port, name: init?.host ? `${d?.name ?? "Database"} on ${init.host}:${init.port}` : base.name });
    setPassword(editing ? null : "");
    setTunnelSecret(editing ? null : "");
    setUrl("");
    setUrlError(null);
    setNameTouched(!!editing);
    setReport(null);
    setError(null);
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
  const missing = missingField(config);
  const valid = missing === null;

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
                    {urlError ?? "Paste a connection URL to fill in the form, or enter the details below."}
                  </div>
                </div>
              )}

              <div className={s.row}>
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
                <Field
                  label="Name"
                  value={config.name}
                  placeholder={effectiveName}
                  onChange={(v) => {
                    setNameTouched(true);
                    set("name", v);
                  }}
                />
              </div>

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
                  <TunnelFields
                    tunnel={config.tunnel ?? null}
                    secret={tunnelSecret}
                    onChange={(tunnel) => set("tunnel", tunnel)}
                    onSecret={(v) => {
                      setReport(null);
                      setTunnelSecret(v);
                    }}
                  />

                  {config.tunnel?.type !== "cloudSql" &&
                    (config.tunnel?.type === "kubernetes" ? (
                      <Field
                        label="Database port in the cluster"
                        mono
                        value={String(config.port || "")}
                        onChange={(v) => set("port", Math.min(Number(v.replace(/\D/g, "")) || 0, 65535))}
                      />
                    ) : (
                      <div className={s.hostRow}>
                        <Field
                          label={config.tunnel ? "Database host (as seen from the tunnel)" : "Host"}
                          mono
                          value={config.host}
                          placeholder={config.tunnel ? "mydb.abc123.eu-west-1.rds.amazonaws.com" : "db.example.com, or a socket folder like /tmp"}
                          onChange={(v) => set("host", v.trim())}
                        />
                        <Field
                          label="Port"
                          mono
                          value={String(config.port || "")}
                          onChange={(v) => set("port", Math.min(Number(v.replace(/\D/g, "")) || 0, 65535))}
                        />
                      </div>
                    ))}

                  <SignInFields
                    config={config}
                    password={password}
                    onConfig={(next) => {
                      setReport(null);
                      setConfig(next);
                    }}
                    onPassword={(v) => {
                      setReport(null);
                      setPassword(v);
                    }}
                  />

                  <div className={s.row}>
                    <Field
                      label="Database name"
                      mono
                      value={config.database ?? ""}
                      placeholder="optional"
                      onChange={(v) => set("database", v.trim() ? v : null)}
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
                  {config.tunnel?.type !== "cloudSql" && (config.sslMode === "verify-full" || config.sslMode === "verify-ca") && (
                    <CaCertField config={config} onChange={(cert) => set("sslRootCert", cert)} />
                  )}
                </>
              )}

              <Segmented<EnvTag> label="Environment" value={config.env} onChange={(v) => set("env", v)} options={ENV_OPTIONS} />

              <div className={s.readOnly}>
                <Switch isSelected={config.readOnly} onChange={(v) => set("readOnly", v)}>
                  Read-only
                </Switch>
                <span className={s.readOnlyHint}>
                  {config.readOnly ? "Data can be viewed but not changed." : "Data on this connection can be changed."}
                </span>
              </div>

              {(report || busy === "test") && (
                <div className={s.steps} aria-live="polite" ref={(el) => el?.scrollIntoView({ block: "nearest" })}>
                  {busy === "test" && (
                    <div className={s.step}>
                      <Spinner />
                      <span>Trying {config.host}:{config.port}…</span>
                    </div>
                  )}
                  {report?.steps.map((step, i) => (
                    <div key={i} className={s.step}>
                      {step.ok ? <CheckIcon className={s.ok} /> : <AlertIcon className={s.fail} />}
                      <span>{step.label}</span>
                      {step.detail && <span className={`${s.stepDetail} selectable`}>{step.detail}</span>}
                      {step.detail?.includes("identity has changed") && config.tunnel?.type === "ssh" && (
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
                {!error && missing && <span className={s.readOnlyHint}>{missing}</span>}
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

const blankSsh = (): TunnelConfig => ({ type: "ssh", host: "", port: 22, user: "", auth: { method: "key", path: "" } });

type Mode = "direct" | "ssh" | "ssm" | "kubernetes" | "cloudSql";

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

/** "Connect through": direct, an SSH bastion, or AWS Systems Manager. */
function TunnelFields({
  tunnel,
  secret,
  onChange,
  onSecret,
}: {
  tunnel: TunnelConfig | null;
  secret: string | null;
  onChange(t: TunnelConfig | null): void;
  onSecret(v: string): void;
}) {
  const mode: Mode = tunnel?.type ?? "direct";
  const [sshHosts, setSshHosts] = useState<SshHost[]>([]);
  useEffect(() => {
    if (mode === "ssh") ipc.sshConfigHosts().then(setSshHosts, () => setSshHosts([]));
  }, [mode]);

  return (
    <div className={s.tunnel}>
      <Segmented<Mode>
        label="Connect through"
        value={mode}
        onChange={(m) => onChange(blankTunnel(m))}
        options={[
          { value: "direct", label: "Direct" },
          { value: "ssh", label: "SSH" },
          { value: "ssm", label: "AWS SSM" },
          { value: "kubernetes", label: "Kubernetes" },
          { value: "cloudSql", label: "Cloud SQL" },
        ]}
      />
      {tunnel?.type === "ssh" && (
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
          <div className={s.hostRow}>
            <Field label="SSH host" mono value={tunnel.host} placeholder="bastion.example.com or 3.120.10.5" onChange={(v) => onChange({ ...tunnel, host: v.trim() })} />
            <Field label="SSH port" mono value={String(tunnel.port || "")} onChange={(v) => onChange({ ...tunnel, port: Number(v.replace(/\D/g, "")) || 0 })} />
          </div>
          <div className={s.row}>
            <Field label="SSH user" mono value={tunnel.user} placeholder="ec2-user or ubuntu" onChange={(v) => onChange({ ...tunnel, user: v.trim() })} />
            <Segmented<"key" | "agent" | "password">
              label="Sign in with"
              value={tunnel.auth.method}
              onChange={(m) => onChange({ ...tunnel, auth: m === "key" ? { method: "key", path: "" } : { method: m } })}
              options={[
                { value: "key", label: "Key file" },
                { value: "agent", label: "SSH agent" },
                { value: "password", label: "Password" },
              ]}
            />
          </div>
          {tunnel.auth.method === "key" && (
            <>
              <div className={s.row}>
                <Field label="Private key" mono value={tunnel.auth.path} placeholder="~/Downloads/my-key.pem" onChange={(v) => onChange({ ...tunnel, auth: { method: "key", path: v } })} />
                <Field
                  label="Key passphrase"
                  type="password"
                  value={secret ?? ""}
                  placeholder={secret === null ? "Saved in Keychain" : "if the key has one"}
                  onChange={onSecret}
                />
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
          {tunnel.auth.method === "password" && (
            <Field label="SSH password" type="password" value={secret ?? ""} placeholder={secret === null ? "Saved in Keychain" : ""} onChange={onSecret} />
          )}
          {tunnel.auth.method === "agent" && <p className={s.tunnelHint}>Uses the keys loaded in your SSH agent (including 1Password and Secretive).</p>}
          <div className={s.readOnly}>
            <Switch isSelected={!!tunnel.jump} onChange={(on) => onChange({ ...tunnel, jump: on ? { host: "", port: 22, user: "" } : null })}>
              Through a jump host first
            </Switch>
            <span className={s.readOnlyHint}>ProxyJump: reach the SSH server via another one</span>
          </div>
          {tunnel.jump && (
            <div className={s.hostRow}>
              <Field label="Jump host" mono value={tunnel.jump.host} placeholder="gateway.example.com" onChange={(v) => onChange({ ...tunnel, jump: { ...tunnel.jump!, host: v.trim() } })} />
              <Field label="Port" mono value={String(tunnel.jump.port || "")} onChange={(v) => onChange({ ...tunnel, jump: { ...tunnel.jump!, port: Math.min(Number(v.replace(/\D/g, "")) || 0, 65535) } })} />
            </div>
          )}
          {tunnel.jump && (
            <Field label="Jump host user" mono value={tunnel.jump.user} placeholder={tunnel.user || "same as the SSH user"} onChange={(v) => onChange({ ...tunnel, jump: { ...tunnel.jump!, user: v.trim() } })} />
          )}
          <p className={s.tunnelHint}>
            Kiyi signs in to the SSH server{tunnel.jump ? " (through the jump host, with the same key)" : ""}, then connects from there to the database host below. For RDS, the database host is the RDS endpoint (…rds.amazonaws.com).
          </p>
        </>
      )}
      {tunnel?.type === "ssm" && (
        <>
          <Field label="EC2 instance ID" mono value={tunnel.target} placeholder="i-0123456789abcdef0" onChange={(v) => onChange({ ...tunnel, target: v.trim() })} />
          <div className={s.row}>
            <Field label="Region" mono value={tunnel.region ?? ""} placeholder="from your AWS config" onChange={(v) => onChange({ ...tunnel, region: v.trim() || null })} />
            <Field label="AWS profile" mono value={tunnel.profile ?? ""} placeholder="default" onChange={(v) => onChange({ ...tunnel, profile: v.trim() || null })} />
          </div>
          <p className={s.tunnelHint}>Needs the AWS CLI and the Session Manager plugin installed, and the instance registered with Systems Manager. No open SSH port required. SSM doesn't use a .pem key: to sign in to an EC2 bastion with one, choose SSH tunnel.</p>
        </>
      )}
      {tunnel?.type === "kubernetes" && (
        <>
          <Field label="Service or pod" mono value={tunnel.target} placeholder="postgres, svc/postgres or pod/postgres-0" onChange={(v) => onChange({ ...tunnel, target: v.trim() })} />
          <div className={s.row}>
            <Field label="Namespace" mono value={tunnel.namespace ?? ""} placeholder="default" onChange={(v) => onChange({ ...tunnel, namespace: v.trim() || null })} />
            <Field label="Context" mono value={tunnel.context ?? ""} placeholder="current kubectl context" onChange={(v) => onChange({ ...tunnel, context: v.trim() || null })} />
          </div>
          <p className={s.tunnelHint}>Runs kubectl port-forward with your kubeconfig. A bare name means a service.</p>
        </>
      )}
      {tunnel?.type === "cloudSql" && (
        <>
          <Field label="Instance connection name" mono value={tunnel.instance} placeholder="my-project:europe-west1:my-db" onChange={(v) => onChange({ ...tunnel, instance: v.trim() })} />
          <p className={s.tunnelHint}>
            Runs the Cloud SQL Auth Proxy (cloud-sql-proxy) with your Google Cloud sign-in (gcloud auth application-default login). The name is on the instance's overview page. The proxy encrypts the connection.
          </p>
        </>
      )}
    </div>
  );
}

/** Database user and how it signs in: a password, or an Amazon RDS IAM token. */
function SignInFields({
  config,
  password,
  onConfig,
  onPassword,
}: {
  config: ConnectionConfig;
  password: string | null;
  onConfig(c: ConnectionConfig): void;
  onPassword(v: string): void;
}) {
  const auth = config.auth ?? { method: "password" };
  return (
    <>
      <div className={s.row}>
        <Field label="User" mono value={config.user} onChange={(v) => onConfig({ ...config, user: v.trim() })} />
        <Segmented<"password" | "awsIam">
          label="Sign in with"
          value={auth.method}
          onChange={(m) => onConfig({ ...config, auth: m === "awsIam" ? { method: "awsIam", region: null, profile: null } : { method: "password" } })}
          options={[
            { value: "password", label: "Password" },
            { value: "awsIam", label: "AWS IAM" },
          ]}
        />
      </div>
      {auth.method === "password" ? (
        <Field label="Password" type="password" value={password ?? ""} placeholder={password === null ? "Saved in Keychain" : ""} onChange={onPassword} />
      ) : (
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
