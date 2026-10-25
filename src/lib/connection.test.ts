import { describe, expect, it } from "vitest";
import { connectionWarnings, missingField, nextMissing, parseSshCommand } from "./connection";
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

describe("nextMissing", () => {
  it("points at the field to fill in next", () => {
    const ssh = { type: "ssh" as const, host: "", port: 22, user: "", auth: { method: "key" as const, path: "" } };
    expect(nextMissing({ ...base, tunnel: ssh })?.field).toBe("sshHost");
    expect(nextMissing({ ...base, tunnel: { ...ssh, host: "3.120.10.5" } })?.field).toBe("sshUser");
    expect(nextMissing({ ...base, tunnel: { ...ssh, host: "3.120.10.5", user: "ec2-user" } })?.field).toBe("sshKey");
    expect(nextMissing({ ...base, user: "" })?.field).toBe("user");
  });
});

describe("connectionWarnings", () => {
  const ssh = { type: "ssh" as const, host: "3.120.10.5", port: 22, user: "ec2-user", auth: { method: "key" as const, path: "/Users/me/Downloads/key.pem" } };

  it("catches an RDS endpoint entered as the SSH host", () => {
    expect(connectionWarnings({ ...base, tunnel: { ...ssh, host: "mydb.abc123.eu-west-1.rds.amazonaws.com" } }).sshHost).toMatch(/RDS endpoint/);
    expect(connectionWarnings({ ...base, tunnel: ssh }).sshHost).toBeUndefined();
  });

  it("catches a public key and a key without a full path", () => {
    expect(connectionWarnings({ ...base, tunnel: { ...ssh, auth: { method: "key", path: "~/.ssh/id_ed25519.pub" } } }).sshKey).toMatch(/public key/);
    expect(connectionWarnings({ ...base, tunnel: { ...ssh, auth: { method: "key", path: "my-key.pem" } } }).sshKey).toMatch(/full path/);
    expect(connectionWarnings({ ...base, tunnel: { ...ssh, auth: { method: "key", path: "C:\\Users\\me\\key.pem" } } }).sshKey).toBeUndefined();
    expect(connectionWarnings({ ...base, tunnel: ssh }).sshKey).toBeUndefined();
  });

  it("suggests localhost when the database host is the SSH server itself", () => {
    expect(connectionWarnings({ ...base, host: "3.120.10.5", tunnel: ssh }).host).toMatch(/localhost/);
    expect(connectionWarnings({ ...base, host: "localhost", tunnel: ssh }).host).toBeUndefined();
  });

  it("suggests SSH for an EC2 server reached directly", () => {
    expect(connectionWarnings({ ...base, host: "ec2-3-120-10-5.eu-central-1.compute.amazonaws.com" }).host).toMatch(/SSH/);
    expect(connectionWarnings({ ...base, host: "mydb.abc123.eu-west-1.rds.amazonaws.com" }).host).toBeUndefined();
  });
});

describe("parseSshCommand", () => {
  it("reads the command AWS shows under Connect → SSH client", () => {
    expect(parseSshCommand('ssh -i "my-key.pem" ec2-user@ec2-3-120-10-5.eu-central-1.compute.amazonaws.com')).toEqual({
      host: "ec2-3-120-10-5.eu-central-1.compute.amazonaws.com",
      user: "ec2-user",
      port: null,
      keyPath: "my-key.pem",
      jump: null,
    });
  });

  it("reads ports, users, jump hosts and skips other options", () => {
    expect(parseSshCommand("ssh -p 2222 -o StrictHostKeyChecking=no -J admin@gw.example.com:2200 -l ubuntu 10.0.1.5")).toEqual({
      host: "10.0.1.5",
      user: "ubuntu",
      port: 2222,
      keyPath: null,
      jump: { host: "gw.example.com", port: 2200, user: "admin" },
    });
    expect(parseSshCommand("ssh -v ubuntu@10.0.1.5")?.user).toBe("ubuntu");
  });

  it("splits a bare user@host and leaves plain host names alone", () => {
    expect(parseSshCommand("ubuntu@3.120.10.5")).toMatchObject({ host: "3.120.10.5", user: "ubuntu" });
    expect(parseSshCommand("3.120.10.5")).toBeNull();
    expect(parseSshCommand("bastion.example.com")).toBeNull();
    expect(parseSshCommand("ssh -i key.pem")).toBeNull();
    expect(parseSshCommand("ubuntu@")).toBeNull();
  });
});
