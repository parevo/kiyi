# Contributing to Kiyi

Thanks for helping. Bug reports, ideas and pull requests are all welcome.

## Reporting a bug

Use **Settings → About & updates → Report a problem** in the app (or ⌘K → "Report a problem"). It opens a new issue with your Kiyi version, system and the end of the log already filled in, so you only need to say what happened. You can also [open an issue](https://github.com/parevo/kiyi/issues/new/choose) directly.

Please don't post passwords, connection strings with passwords in them, or data you can't share. Kiyi's log never contains passwords, keys or SQL text, but check before you submit.

Security problems go to the address in [SECURITY.md](SECURITY.md), not to a public issue.

## Suggesting a feature

Open an issue describing what you were trying to do and where Kiyi got in the way. "I wanted to X, and had to Y" helps more than a solution on its own.

## Working on the code

The [README](README.md#development) explains the layout, how to run the app and the test databases, and how to run the tests. In short:

```sh
pnpm install
./dev/setup-ssh-key.sh
docker compose -f dev/docker-compose.yml up -d --wait
pnpm tauri dev
```

Before opening a pull request, run what CI runs:

```sh
pnpm typecheck && pnpm test && pnpm vite build
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
KIYI_LIVE=1 cargo test -p kiyi-core --test live --test objects   # with the Docker databases up
```

### How the code is shaped

- **Logic lives in `crates/kiyi-core`.** It has no Tauri dependency. `src-tauri` is a thin layer of commands over it, and `crates/kiyi-devbridge` mirrors those commands over HTTP for browser testing. A new command goes in all three, plus `src/lib/ipc.ts`.
- **Every SQL string Kiyi writes is shown to the person before it runs**, and is built through `dialect.rs` for quoting. Nothing hides behind bind parameters.
- **Changes that write go through review.** Production connections are read-only by default, destructive changes are explained in plain words, and generated SQL opens in the editor rather than running behind someone's back.
- **Words matter.** Kiyi is for people who don't write SQL. Prefer "Money / exact number" to `numeric(12,2)` in the interface, explain errors in plain language, and keep the technical details one click away for those who want them.
- **Test against real servers.** Driver and SQL changes should have a live test in `crates/kiyi-core/tests` that runs against the Docker databases.

### Pull requests

- Keep each pull request to one change, and describe what it does for someone using Kiyi.
- Include screenshots for interface changes, in both light and dark themes when colors are involved.
- New engines plug in through a driver in `crates/kiyi-core/src/drivers` and an entry in `crates/kiyi-core/src/catalog.rs`.

By contributing, you agree that your contributions are licensed under the [MIT License](LICENSE).
