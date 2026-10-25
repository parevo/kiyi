import type { FieldId } from "./connection";

/** What a field wants, and where on AWS to find it, shown under the field being filled in. */
export interface FieldGuide {
  help: string;
  aws?: string;
}

const RDS_ENDPOINT = "RDS console → Databases → your database → Connectivity & security → Endpoint.";

export const GUIDE: Record<FieldId | "hostViaTunnel" | "password" | "database" | "ssmRegion" | "ssmProfile" | "k8sNamespace" | "k8sContext", FieldGuide> = {
  file: { help: "The database file on this computer, or a new one to create." },
  host: {
    help: "The server's address, like db.example.com or 203.0.113.5. For a database on this computer or in Docker: localhost.",
    aws: RDS_ENDPOINT,
  },
  hostViaTunnel: {
    help: "The database's address as the server in step 1 sees it. If the database runs on that same server, keep localhost.",
    aws: `For RDS, the endpoint: ${RDS_ENDPOINT}`,
  },
  port: { help: "Usually the default for the database type (PostgreSQL 5432, MySQL 3306, SQL Server 1433). Change it only if the server uses another.", aws: "Shown next to the RDS endpoint." },
  user: { help: "The database user. This isn't the user you sign in to a server with.", aws: "RDS: Configuration tab → Master username, or a user created in the database." },
  password: { help: "The database user's password. Kiyi keeps it in your system keychain.", aws: "RDS: the master password chosen when the database was created. RDS can't show it again; set a new one with Modify." },
  database: { help: "Which database on the server to open: the part after the last / in a connection string. Ask whoever set it up if you're not sure." },
  sshHost: {
    help: "The address of a server you can SSH into that can reach the database. Have an ssh command (ssh -i key.pem user@host)? Paste it here to fill in this step.",
    aws: "EC2 console → Instances → your instance → Public IPv4 address. Or Connect → SSH client, and paste the command shown there.",
  },
  sshPort: { help: "22, unless the server was set up with another SSH port." },
  sshUser: { help: "The user you sign in to the server with.", aws: "ec2-user on Amazon Linux, ubuntu on Ubuntu, admin on Debian." },
  sshKey: { help: "The private key file for that server.", aws: "The .pem file downloaded when the EC2 key pair was created, usually in Downloads. The key pair's name is on the instance's Details tab." },
  jumpHost: { help: "A gateway server to go through before the SSH server (ProxyJump in ~/.ssh/config)." },
  ssmTarget: { help: "The EC2 instance to go through. It needs the SSM agent and an IAM role for Systems Manager.", aws: "EC2 console → Instances → Instance ID column (starts with i-)." },
  ssmRegion: { help: "Leave empty to use the region from your AWS config.", aws: "The region shown at the top right of the AWS console, like eu-central-1." },
  ssmProfile: { help: "A profile name from ~/.aws/config. Leave empty for the default one." },
  k8sTarget: { help: "The database's service or pod. List them with kubectl get svc, or kubectl get pods." },
  k8sNamespace: { help: "Leave empty for the default namespace. List them with kubectl get namespaces." },
  k8sContext: { help: "Which cluster, as named in your kubeconfig. Leave empty for the current one (kubectl config current-context)." },
  cloudSqlInstance: { help: "Cloud SQL console → your instance → Overview → Connection name, in the form project:region:instance." },
};

export type Mode = "direct" | "ssh" | "ssm" | "kubernetes" | "cloudSql";

/** What each way of reaching the database means, shown under the choice. */
export const MODE_HELP: Record<Mode, string> = {
  direct: "The database accepts connections from this computer: it runs here, in Docker, or is open to the internet.",
  ssh: "The database is private. Kiyi signs in to a server you can SSH into (on AWS, an EC2 instance or bastion) and reaches the database from there.",
  ssm: "Like SSH, but through AWS Systems Manager: no open SSH port or .pem key, only the AWS CLI and the Session Manager plugin.",
  kubernetes: "The database runs in a Kubernetes cluster. Kiyi uses kubectl port-forward with your kubeconfig.",
  cloudSql: "A Google Cloud SQL database, through the Cloud SQL Auth Proxy and your gcloud sign-in.",
};
