import type { ConnectionConfig, JumpHost } from "./types";

/** The form fields Kiyi can point at when guiding someone through a connection. */
export type FieldId =
  | "file"
  | "sshHost"
  | "sshPort"
  | "sshUser"
  | "sshKey"
  | "jumpHost"
  | "ssmTarget"
  | "k8sTarget"
  | "cloudSqlInstance"
  | "host"
  | "port"
  | "user";

/** The first thing the form still needs, and which field it belongs to, or `null` when it can be tested. */
export function nextMissing(c: ConnectionConfig): { field: FieldId; message: string } | null {
  const need = (field: FieldId, message: string) => ({ field, message });
  if (c.kind === "sqlite") return c.database?.trim() ? null : need("file", "Choose a database file.");
  const t = c.tunnel;
  if (t?.type === "ssh") {
    if (!t.host.trim()) return need("sshHost", "Enter the SSH host (your bastion or EC2 address).");
    if (!t.port) return need("sshPort", "Enter the SSH port.");
    if (!t.user.trim()) return need("sshUser", "Enter the SSH user (ec2-user on Amazon Linux, ubuntu on Ubuntu).");
    if (t.auth.method === "key" && !t.auth.path.trim()) return need("sshKey", "Choose the private key file.");
    if (t.jump && !t.jump.host.trim()) return need("jumpHost", "Enter the jump host, or turn it off.");
  }
  if (t?.type === "ssm" && !t.target.trim()) return need("ssmTarget", "Enter the EC2 instance ID.");
  if (t?.type === "kubernetes" && !t.target.trim()) return need("k8sTarget", "Enter the Kubernetes service or pod.");
  if (t?.type === "cloudSql" && t.instance.split(":").length !== 3) return need("cloudSqlInstance", "Enter the instance connection name (project:region:instance).");
  if (t?.type !== "cloudSql" && t?.type !== "kubernetes" && !c.host.trim()) return need("host", "Enter the database host.");
  if (t?.type !== "cloudSql" && !c.port) return need("port", "Enter the port.");
  if (!c.user.trim()) return need("user", "Enter the database user.");
  return null;
}

/** Why the form can't be tested or saved yet, or `null` when it can. */
export function missingField(c: ConnectionConfig): string | null {
  return nextMissing(c)?.message ?? null;
}

const RDS = /\.rds\.amazonaws\.com\.?$/i;
const EC2 = /(^ec2-[\d-]+\..*\.amazonaws\.com\.?$)|(\.compute(-1)?\.amazonaws\.com\.?$)/i;
const ABSOLUTE = /^(\/|~|[a-z]:[\\/]|\\\\)/i;

/** Values that look like they went into the wrong field, with what to do instead. */
export function connectionWarnings(c: ConnectionConfig): Partial<Record<FieldId, string>> {
  const out: Partial<Record<FieldId, string>> = {};
  const host = c.host.trim();
  const t = c.tunnel;
  if (t?.type === "ssh") {
    if (RDS.test(t.host.trim())) out.sshHost = "That's a database address (an RDS endpoint). Enter it as the database host in the next step, and the EC2 server's address here.";
    if (t.auth.method === "key") {
      const path = t.auth.path.trim();
      if (/\.pub$/i.test(path)) out.sshKey = "That's the public key. Choose the private one: the same file without .pub.";
      else if (path && !ABSOLUTE.test(path)) out.sshKey = "Kiyi needs the full path to the key. Use Choose key file… to pick it.";
    }
    if (host && host === t.host.trim()) out.host = "This is the SSH server's own address. If the database runs on that server, enter localhost.";
  }
  if (!t && EC2.test(host)) out.host = "This is an EC2 server. If its database isn't open to the internet, choose SSH above and enter this address as the SSH host.";
  return out;
}

export interface SshCommand {
  host: string;
  user: string | null;
  port: number | null;
  keyPath: string | null;
  jump: JumpHost | null;
}

/** `user@host[:port]` */
function destination(text: string): { host: string; user: string | null; port: number | null } | null {
  const m = /^(?:([^@\s]+)@)?(\[[^\]]+\]|[^:@\s]+)(?::(\d+))?$/.exec(text);
  if (!m) return null;
  return { user: m[1] ?? null, host: m[2].replace(/^\[|\]$/g, ""), port: m[3] ? Number(m[3]) : null };
}

/**
 * Reads an `ssh` command, such as the one AWS shows under EC2 → Connect → SSH client
 * (`ssh -i "my-key.pem" ec2-user@ec2-3-120-10-5.eu-central-1.compute.amazonaws.com`), or a bare
 * `user@host`. Returns `null` for anything else, so plain host names are left alone.
 */
export function parseSshCommand(text: string): SshCommand | null {
  const trimmed = text.trim();
  const words = [...trimmed.matchAll(/"([^"]*)"|'([^']*)'|(\S+)/g)].map((m) => m[1] ?? m[2] ?? m[3]);
  if (words[0] !== "ssh") {
    if (words.length !== 1 || !trimmed.includes("@")) return null;
    const d = destination(trimmed);
    return d && { ...d, keyPath: null, jump: null };
  }
  let keyPath: string | null = null;
  let port: number | null = null;
  let user: string | null = null;
  let jump: JumpHost | null = null;
  let target: string | null = null;
  for (let i = 1; i < words.length; i++) {
    const w = words[i];
    const next = () => words[++i] ?? "";
    if (w === "-i") keyPath = next();
    else if (w === "-p") port = Number(next()) || null;
    else if (w === "-l") user = next();
    else if (w === "-J") {
      const j = destination(next().split(",")[0]);
      if (j) jump = { host: j.host, port: j.port ?? 22, user: j.user ?? "" };
    } else if (/^-[46AaCfGgKkMNnqsTtVvXxYy]+$/.test(w)) continue;
    else if (w.startsWith("-")) next();
    else if (!target) target = w;
  }
  const d = target && destination(target);
  if (!d) return null;
  return { host: d.host, user: d.user ?? user, port: port ?? d.port, keyPath, jump };
}
