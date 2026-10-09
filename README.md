<p align="center">
  <img src="site/assets/icon.svg" width="84" height="84" alt="" />
</p>

<h1 align="center">Kiyi</h1>

<p align="center">
  <b>Your database, finally friendly.</b><br />
  Browse, search and edit PostgreSQL, MySQL, MariaDB and SQLite like a spreadsheet, and ask for data in plain words.
</p>

<p align="center">
  <a href="https://parevo.github.io/kiyi/">Website</a> ·
  <a href="https://github.com/parevo/kiyi/releases/latest">Download</a> ·
  <a href="https://github.com/parevo/kiyi/issues">Report an issue</a>
</p>

<p align="center">
  <a href="https://github.com/parevo/kiyi/releases/latest"><img src="https://img.shields.io/github/v/release/parevo/kiyi?label=release&color=4fb3a9" alt="Latest release" /></a>
  <a href="https://github.com/parevo/kiyi/actions/workflows/ci.yml"><img src="https://github.com/parevo/kiyi/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-d8b878" alt="MIT license" /></a>
</p>

<picture>
  <source media="(prefers-color-scheme: light)" srcset="site/assets/shots/data-light.webp" />
  <img src="site/assets/shots/data-dark.webp" alt="Kiyi showing a customers table with a row open in the details panel" />
</picture>

## Why Kiyi

Most database tools are built for people who already think in SQL. Kiyi is built for everyone else, and stays out of the way of the people who do.

- **Edit like a spreadsheet.** Change a cell and it's saved, with Undo right there. Insert rows with a form that shows what's required and what each default will be.
- **Ask AI.** Type "pro customers in Germany who spent over 1k" and get filters you can see, tweak and remove. Bring any provider: Anthropic, OpenAI, Gemini, xAI Grok, OpenRouter, Groq, Mistral, or a local model with Ollama or LM Studio. Only the table's structure is sent, never your rows.
- **Design tables without DDL.** Types in plain terms ("Money / exact number"), required and unique rules, defaults, relationships and indexes. Kiyi writes the right `ALTER` statements for your database, in the right order.
- **Reach private databases.** SSH tunnels through a bastion (password, key file or ssh-agent, hosts picked from `~/.ssh/config`, jump hosts), AWS SSM port forwarding, Kubernetes `port-forward` and the Cloud SQL Auth Proxy, with every step of the connection test explained. Sign in to RDS with IAM, verify SSL against your own CA (the Amazon RDS bundle is fetched for you), or connect over a Unix socket. Databases running locally or in Docker are found automatically.
- **Safe on production.** Production connections are read-only by default, edits wait for an explicit Save, and destructive changes are explained in plain language before they run.
- **Ask questions, get charts.** "Monthly revenue this year" becomes a query you can see, run as a chart. AI also writes, explains and fixes SQL in the editor.
- **Charts and summaries.** Any result as bars, a line, a pie or a single figure; group a table by any column (dates by day, month or year) without SQL.
- **Import and export.** Excel, CSV and JSON out, from any view or query; Excel and CSV in (Turkish and other Excel encodings detected), batched, all or nothing.
- **Work faster.** ⌘K to jump anywhere, query history and saved queries, saved table views, hidden/frozen/reordered columns, copy rows as CSV, Markdown, JSON or INSERTs, find and replace across a column.
- **See and protect the whole database.** A schema diagram, readable query plans, backups and restores, and comparing two databases with the SQL that would align them.
- **Developer mode.** The SQL behind every action, raw column types, and a SQL editor with schema-aware autocomplete.

Passwords and API keys live in the system keychain. No account, no telemetry.

## Fast

| | |
| --- | --- |
| Download | 8.6 MB |
| Launch to window | 0.31 s |
| Memory at idle | 79 MB |
| Open, sort or filter a 1M-row table | ~1 ms |
| Read all 1M rows | 0.77 s on PostgreSQL, 0.68 s on MySQL (faster than `psql` and `mysql` themselves) |

Measured on an M1 Pro MacBook Pro. Full results and how to reproduce them: [docs/benchmarks.md](docs/benchmarks.md).

## Install

Download the latest version for **macOS** (Apple Silicon or Intel) or **Windows** (64-bit) from the [website](https://parevo.github.io/kiyi/#download) or the [releases page](https://github.com/parevo/kiyi/releases/latest). Kiyi updates itself in the background and asks before restarting.

> The macOS app is signed with a Developer ID and notarized by Apple. The Windows installer isn't code-signed yet: if SmartScreen asks, choose **More info → Run anyway**.

## Supported databases

| Database   | Status      |
| ---------- | ----------- |
| PostgreSQL | Supported   |
| MySQL      | Supported   |
| MariaDB    | Supported   |
| SQLite     | Supported   |
| SQL Server | Supported (2017 and later, Azure SQL) |

New engines plug in through a driver and an entry in `crates/kiyi-core/src/catalog.rs`; the app itself doesn't change.

## Development

Kiyi is a Rust core with a Tauri 2 shell and a React interface.

```
crates/kiyi-core/       Drivers, database catalog, SQL planners, tunnels, AI, connection store, keychain. No Tauri dependency.
crates/kiyi-devbridge/  The core over HTTP, so the UI can run in a plain browser for screenshots and end-to-end tests.
src-tauri/              Tauri commands (a thin layer over the core) and the updater.
src/                    React + React Aria, CSS Modules, CodeMirror 6, Glide Data Grid.
src/styles/tokens.css   Every color, spacing value and font.
site/                   The website, deployed to GitHub Pages.
```

### Prerequisites

Rust (stable), Node.js 24, pnpm, and Docker for the test databases.

### Run the app

```sh
pnpm install
./dev/setup-ssh-key.sh                                  # local key for the test SSH bastion
docker compose -f dev/docker-compose.yml up -d --wait   # PostgreSQL, MySQL and an SSH bastion
pnpm tauri dev
```

Test connections:

| URL                                         | Notes                                    |
| ------------------------------------------- | ---------------------------------------- |
| `postgres://kiyi:kiyi@localhost:55432/shop` | Sample data to play with                 |
| `mysql://kiyi:kiyi@localhost:53306/shop`    | Sample data to play with                 |
| `postgres://kiyi:kiyi@localhost:55432/demo` | The "Acme Store" used for screenshots    |

To try an SSH tunnel, use host `127.0.0.1`, port `52222`, user and password `kiyi`, and `postgres:5432` as the database host.

### SQL Server

SQL Server is optional in the dev setup (it's large and emulated on Apple Silicon):

```sh
docker compose -f dev/docker-compose.yml --profile sqlserver up -d --wait
docker compose -f dev/docker-compose.yml exec sqlserver /opt/mssql-tools18/bin/sqlcmd -C -S localhost -U sa -P Kiyi_pass1 -i /seed/sqlserver.sql
KIYI_LIVE_SQLSERVER=1 cargo test -p kiyi-core --test sqlserver_live
```

Connect in the app with `sqlserver://sa:Kiyi_pass1@localhost:51433/kiyi_test`. SQL Server has no read-only session setting, so Kiyi enforces read-only connections itself.

### Tests

```sh
cargo test --workspace                           # unit tests
KIYI_LIVE=1 cargo test -p kiyi-core --test live  # drivers, tunnels, import/export against the Docker databases
pnpm typecheck
pnpm test                                        # interface logic (Vitest)
cargo audit                                      # known vulnerabilities; accepted ones are explained in .cargo/audit.toml
```

Live tests use a separate `kiyi_test` database, so editing `shop` in the app never breaks them.

### UI in the browser

`kiyi-devbridge` serves the core over HTTP and the UI talks to it instead of Tauri (`src/dev/bridge.ts`). Useful for screenshots and scripted flows with Playwright:

```sh
pnpm dev                                  # Vite on :1420
cargo run -p kiyi-devbridge               # bridge on :1421
node dev/shoot.mjs /tmp/shots dark        # screenshots of the main screens
node dev/flows.mjs /tmp/flows             # end-to-end flows against the real databases
node dev/marketing.mjs site/assets/shots  # website screenshots, dark and light
```

To try the live AI test (it spends a few tokens): `XAI_API_KEY=… KIYI_LIVE=1 cargo test -p kiyi-core --test ai_live -- --nocapture`.

### AI providers

Providers are managed in **Settings → AI**. Kiyi speaks the Anthropic Messages API and any OpenAI-compatible `/chat/completions` endpoint. Keys are stored in the keychain; without one, the provider's environment variable is used (for example `ANTHROPIC_API_KEY`). Whatever the model returns is validated as a single read-only condition before it runs.

### Troubleshooting

Kiyi writes a log to `~/Library/Logs/com.parevo.kiyi/kiyi.log` on macOS and `%LOCALAPPDATA%\com.parevo.kiyi\logs\kiyi.log` on Windows. **Settings → About & updates → Copy diagnostics** copies the version, platform and recent log for a bug report. SQL text, passwords and keys are never logged.

## Releases

- Every push to `main` builds a beta (`vX.Y.Z-beta.N`) for users on the beta channel.
- Pushing a `vX.Y.Z` tag builds a stable release for everyone.

```sh
git tag v0.2.0 && git push origin v0.2.0
```

The app checks for updates at launch and every four hours, downloads in the background, and installs when the user chooses to restart.

Required repository secrets: `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` for update signatures. macOS builds are signed and notarized when `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD` and `APPLE_SIGNING_IDENTITY` are set, together with an App Store Connect API key (`APPLE_API_KEY`, `APPLE_API_ISSUER`, `APPLE_API_PRIVATE_KEY`) or an Apple ID (`APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`). Without them the app is ad-hoc signed.

## License

[MIT](LICENSE) © Parevo
