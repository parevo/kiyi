import type { ConnectionConfig } from "./types";

/** Why the form can't be tested or saved yet, or `null` when it can. */
export function missingField(c: ConnectionConfig): string | null {
  if (c.kind === "sqlite") return c.database?.trim() ? null : "Choose a database file.";
  const t = c.tunnel;
  if (t?.type === "ssh") {
    if (!t.host.trim()) return "Enter the SSH host (your bastion or EC2 address).";
    if (!t.port) return "Enter the SSH port.";
    if (!t.user.trim()) return "Enter the SSH user (ec2-user on Amazon Linux, ubuntu on Ubuntu).";
    if (t.auth.method === "key" && !t.auth.path.trim()) return "Choose the private key file.";
    if (t.jump && !t.jump.host.trim()) return "Enter the jump host, or turn it off.";
  }
  if (t?.type === "ssm" && !t.target.trim()) return "Enter the EC2 instance ID.";
  if (t?.type === "kubernetes" && !t.target.trim()) return "Enter the Kubernetes service or pod.";
  if (t?.type === "cloudSql" && t.instance.split(":").length !== 3) return "Enter the instance connection name (project:region:instance).";
  if (t?.type !== "cloudSql" && t?.type !== "kubernetes" && !c.host.trim()) return "Enter the database host.";
  if (t?.type !== "cloudSql" && !c.port) return "Enter the port.";
  if (!c.user.trim()) return "Enter the database user.";
  return null;
}
