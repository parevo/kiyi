# Benchmarks

Two sets of numbers, both reproducible on your own machine. Results are in [`docs/benchmarks.md`](../../docs/benchmarks.md).

## 1. What Kiyi does with your data

`crates/kiyi-core/examples/bench.rs` times what happens when you use the app (open a table, sort, filter, search, jump deep into it, stream a whole table, export it) against a 1,000,000-row table with mixed types, through the same code the app runs: SQL generation, driver, value decoding, and the JSON encoding the UI receives.

```sh
docker compose -f dev/docker-compose.yml up -d --wait

# Seed the 1M-row `kiyi_bench` databases (about a minute).
docker exec dev-postgres-1 psql -U kiyi -d postgres -c "DROP DATABASE IF EXISTS kiyi_bench" -c "CREATE DATABASE kiyi_bench"
docker exec -i dev-postgres-1 psql -U kiyi -d kiyi_bench -q < dev/bench/seed-postgres.sql
docker exec dev-mysql-1 mysql -uroot -pkiyi -e "DROP DATABASE IF EXISTS kiyi_bench; CREATE DATABASE kiyi_bench; GRANT ALL ON kiyi_bench.* TO 'kiyi'@'%'"
docker exec -i dev-mysql-1 mysql -ukiyi -pkiyi kiyi_bench < dev/bench/seed-mysql.sql

cargo run --release -p kiyi-core --example bench -- bench.json
```

Each scenario runs once to warm up and then 7 times (3 for full scans and exports); the median is reported.

For context, the same full read with the databases' own command-line clients, run inside the containers (no network hop, output discarded):

```sh
time docker exec dev-postgres-1 sh -c 'psql -U kiyi -d kiyi_bench -qAt -c "select * from events" > /dev/null'
time docker exec dev-mysql-1 sh -c 'mysql -ukiyi -pkiyi kiyi_bench -B -e "select * from events" > /dev/null 2>&1'
```

## 2. The app on your Mac

`dev/bench/apps.py` launches the app and measures:

- **Launch:** from starting the binary to its first on-screen window of at least 700×440 points, so splash screens don't count. One warm-up launch first (first-run setup, Gatekeeper), then the median of 3.
- **Memory:** after 15 idle seconds with no connection open, the physical footprint (Activity Monitor's "Memory") summed over the app and every process working for it: child processes (Electron helpers) and the WebKit processes started on its behalf (Tauri).
- **Size:** the installed `.app` bundle.

```sh
swiftc -O dev/bench/winwait.swift -o /tmp/winwait
python3 dev/bench/apps.py /tmp/winwait apps.json "Kiyi=/Applications/Kiyi.app"
```

It takes any number of `Name=/path/App.app` arguments, so other apps can be measured the same way. Use native builds (on Apple Silicon, check with `lipo -archs App.app/Contents/MacOS/*`); an Intel build under Rosetta is measured unfairly.

"First window" is when the window is on screen; the app may still be filling it in at that moment.
