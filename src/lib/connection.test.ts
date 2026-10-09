import { describe, expect, it } from "vitest";
import { missingField } from "./connection";
import type { ConnectionConfig } from "./types";

const base: ConnectionConfig = {
  id: "",
  name: "",
  kind: "postgres",
  host: "db.example.com",
  port: 5432,
  user: "app",
  database: null,
  sslMode: "prefer",
  env: "local",
  readOnly: false,
};

describe("missingField", () => {
  it("accepts a complete direct connection", () => {
    expect(missingField(base)).toBeNull();
  });

  it("names the first thing missing", () => {
    expect(missingField({ ...base, host: " " })).toMatch(/host/);
    expect(missingField({ ...base, user: "" })).toMatch(/user/);
    expect(missingField({ ...base, kind: "sqlite", database: null })).toMatch(/file/);
  });

  it("checks the SSH tunnel, including the key file and jump host", () => {
    const ssh = { type: "ssh" as const, host: "bastion", port: 22, user: "ec2-user", auth: { method: "key" as const, path: "" } };
    expect(missingField({ ...base, tunnel: ssh })).toMatch(/private key/);
    expect(missingField({ ...base, tunnel: { ...ssh, auth: { method: "agent" } } })).toBeNull();
    expect(missingField({ ...base, tunnel: { ...ssh, auth: { method: "agent" }, jump: { host: "", port: 22, user: "" } } })).toMatch(/jump host/);
  });

  it("doesn't ask for a host when the tunnel picks the database", () => {
    expect(missingField({ ...base, host: "", tunnel: { type: "cloudSql", instance: "p:r:i" } })).toBeNull();
    expect(missingField({ ...base, host: "", tunnel: { type: "cloudSql", instance: "nope" } })).toMatch(/project:region:instance/);
    expect(missingField({ ...base, host: "", tunnel: { type: "kubernetes", target: "svc/pg", namespace: null, context: null } })).toBeNull();
  });
});
