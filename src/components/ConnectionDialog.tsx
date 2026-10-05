import { useEffect, useState } from "react";
import { Dialog, Heading, Input, Modal, ModalOverlay, TextField } from "react-aria-components";
import { errorMessage, ipc } from "../lib/ipc";
import type { ConnectionConfig, DriverInfo, EnvTag, SslMode, TestReport } from "../lib/types";
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
  sslMode: "disable",
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
      setConfig((c) => ({ ...parsed.config, id: c.id, name: nameTouched ? c.name : parsed.config.name }));
      if (parsed.password !== null) setPassword(parsed.password);
    } catch (e) {
      setUrlError(errorMessage(e));
    }
  };

  const effectiveName = config.name.trim() || `${config.database ?? config.user} @ ${config.host}`;
  const valid = config.host.trim() !== "" && config.user.trim() !== "" && config.port > 0;

  const test = async () => {
    setBusy("test");
    setReport(await ipc.testConnection(config, password));
    setBusy(null);
  };

  const submit = async () => {
    setBusy("save");
    setError(null);
    try {
      const saved = await save({ ...config, name: effectiveName }, password);
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

              <div className={s.hostRow}>
                <Field label="Host" mono value={config.host} onChange={(v) => set("host", v)} />
                <Field
                  label="Port"
                  mono
                  value={String(config.port || "")}
                  onChange={(v) => set("port", Number(v.replace(/\D/g, "")) || 0)}
                />
              </div>

              <div className={s.row}>
                <Field label="User" mono value={config.user} onChange={(v) => set("user", v)} />
                <Field
                  label="Password"
                  type="password"
                  value={password ?? ""}
                  placeholder={password === null ? "Saved in Keychain" : ""}
                  onChange={(v) => {
                    setReport(null);
                    setPassword(v);
                  }}
                />
              </div>

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
                  value={config.sslMode}
                  onChange={(v) => set("sslMode", v)}
                  options={[
                    { value: "disable", label: "Off" },
                    { value: "prefer", label: "Prefer" },
                    { value: "require", label: "Require" },
                    { value: "verify-full", label: "Verify" },
                  ]}
                />
              </div>

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
                <div className={s.steps} aria-live="polite">
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
