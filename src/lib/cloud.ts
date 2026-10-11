import type { DbKind, SslMode } from "./types";

/** Hosted databases people connect to most, with what trips them up. */
export interface CloudProvider {
  id: string;
  name: string;
  /** Catalog driver id the preset selects. */
  driver: "postgres" | "mysql" | "sqlserver";
  port: number;
  ssl: SslMode;
  /** Where the connection details are in the provider's dashboard. */
  where: string;
  tip?: string;
  hosts: RegExp;
}

export const CLOUD: CloudProvider[] = [
  {
    id: "supabase",
    name: "Supabase",
    driver: "postgres",
    port: 5432,
    ssl: "require",
    where: "Project → Connect → Connection string → URI",
    tip: "On networks without IPv6, use the Session pooler string (aws-0-….pooler.supabase.com).",
    hosts: /\.supabase\.(co|com)$/i,
  },
  { id: "neon", name: "Neon", driver: "postgres", port: 5432, ssl: "require", where: "Project dashboard → Connect → Connection string", hosts: /\.neon\.tech$/i },
  {
    id: "planetscale",
    name: "PlanetScale",
    driver: "mysql",
    port: 3306,
    ssl: "require",
    where: "Database → Connect → Create password → General",
    tip: "PlanetScale passwords are shown once; paste the whole connection string here.",
    hosts: /\.psdb\.cloud$/i,
  },
  {
    id: "rds",
    name: "Amazon RDS / Aurora",
    driver: "postgres",
    port: 5432,
    ssl: "require",
    where: "RDS console → Databases → your database → Connectivity & security → Endpoint",
    tip: "Not publicly accessible? Connect through SSH or AWS SSM below. Choose Verify to check the certificate; Kiyi fetches Amazon's bundle.",
    hosts: /\.rds\.amazonaws\.com$/i,
  },
  {
    id: "azuresql",
    name: "Azure SQL",
    driver: "sqlserver",
    port: 1433,
    ssl: "require",
    where: "Azure portal → SQL database → Connection strings → ADO.NET",
    tip: "Allow your IP address under Networking on the server first.",
    hosts: /\.database\.windows\.net$/i,
  },
  {
    id: "azuredb",
    name: "Azure Database for PostgreSQL / MySQL",
    driver: "postgres",
    port: 5432,
    ssl: "require",
    where: "Azure portal → your server → Connect",
    hosts: /\.(postgres|mysql)\.database\.azure\.com$/i,
  },
  {
    id: "digitalocean",
    name: "DigitalOcean",
    driver: "postgres",
    port: 25060,
    ssl: "require",
    where: "Databases → your cluster → Connection details → Connection string",
    tip: "Add this computer under Trusted sources, or the connection is refused.",
    hosts: /\.db\.ondigitalocean\.com$/i,
  },
  { id: "railway", name: "Railway", driver: "postgres", port: 5432, ssl: "prefer", where: "Service → Variables → DATABASE_PUBLIC_URL", hosts: /\.(railway\.app|rlwy\.net|proxy\.rlwy\.net)$/i },
  { id: "render", name: "Render", driver: "postgres", port: 5432, ssl: "require", where: "Database → Info → External Database URL", hosts: /\.render\.com$/i },
  { id: "aiven", name: "Aiven", driver: "postgres", port: 5432, ssl: "require", where: "Service → Overview → Connection information → Service URI", hosts: /\.aivencloud\.com$/i },
];

export const providerById = (id: string | undefined) => CLOUD.find((p) => p.id === id);

/** The hosted provider a host name belongs to, if Kiyi knows it. */
export const providerForHost = (host: string) => CLOUD.find((p) => p.hosts.test(host.trim()));

/** Google Cloud SQL is reached through its proxy, a separate path in the form. */
export const kindOf = (p: CloudProvider): DbKind => (p.driver === "postgres" ? "postgres" : p.driver === "mysql" ? "mysql" : "sqlserver");
