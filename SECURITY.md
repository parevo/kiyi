# Security

Kiyi holds the keys to people's databases, so security reports get priority.

## Reporting a vulnerability

Please **don't open a public issue**. Report it privately through [GitHub's private vulnerability reporting](https://github.com/parevo/kiyi/security/advisories/new) for this repository.

Include what an attacker could do, the steps to reproduce it, and the Kiyi version (Settings → About & updates). You'll get a reply within a few days, and a fix is released as soon as it's ready, with credit to you unless you'd rather stay anonymous.

## Supported versions

Security fixes go into the latest release. Kiyi updates itself, so staying current is the default.

## How Kiyi protects your data

- **Passwords, SSH passphrases and API keys** are stored in the system keychain (macOS Keychain, Windows Credential Manager), never in Kiyi's files, logs or exported connection files.
- **No telemetry and no account.** Kiyi talks only to your databases, the AI provider you choose, and GitHub to check for updates. Problem reports open in your browser and are sent only if you submit them.
- **AI requests contain table and column names, not row data.** The only exception is planning a data move, where the person can choose to show the AI three example rows per source table; it's off by default. Anything the AI writes is checked before it runs: filters must parse as a single read-only condition, questions as a single read-only query, and migration plans may only name tables and columns that exist.
- **SSH host keys** are remembered and checked on every connection.
- **Production connections** are read-only by default, enforced by Kiyi itself as well as by the database session.
- **Updates are signed**, and the macOS app is signed and notarized by Apple.
