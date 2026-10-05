# Benchmarks

Measured on 2026-10-05 with Kiyi 0.1.0 on a MacBook Pro (M1 Pro, 16 GB, macOS 26.6), PostgreSQL 17.11 and MySQL 8.4.11 running locally in Docker. How to reproduce: [`dev/bench/README.md`](../dev/bench/README.md).

## The app

The released `Kiyi_0.1.0_aarch64.dmg`, median of 3 launches after a warm-up.

| | |
| --- | --- |
| Download | **8.6 MB** |
| Installed size | **18 MB** |
| Launch to window | **0.31 s** |
| Memory at idle (app + WebKit processes) | **79 MB** |

## Working with a 1,000,000-row table

A table with eight mixed columns (integers, text, decimal, boolean, timestamp, JSON) and an index on the timestamp. Each step goes through the same code the app runs, from SQL generation to the values the UI receives. Median of 7 runs (3 for full reads and exports).

| | PostgreSQL | MySQL |
| --- | ---: | ---: |
| Connect | 8.7 ms | 4.3 ms |
| Open a table (first 300 rows) | 1.1 ms | 0.8 ms |
| Sort by date | 1.4 ms | 0.9 ms |
| Two filters (`kind = purchase`, `amount > 500`) | 1.7 ms | 2.8 ms |
| Table structure | 4.0 ms | 3.6 ms |
| Exact row count | 20 ms | 57 ms |
| Jump to row 500,000 | 37 ms | 124 ms |
| Search text in every row | 245 ms | 435 ms |
| Stream all 1M rows: first row | 0.7 ms | 1.1 ms |
| Stream all 1M rows: done | **0.77 s** (1.29M rows/s) | **0.68 s** (1.48M rows/s) |
| Export 1M rows to CSV | 1.5 s | 1.3 s |

For context, the databases' own command-line clients reading the same million rows inside their containers, with no network hop and output thrown away:

| | PostgreSQL (`psql`) | MySQL (`mysql`) |
| --- | ---: | ---: |
| Read 1M rows | 0.92 s | 0.98 s |

Kiyi reads over the network, decodes every value and encodes it for the interface, and still finishes first.

## Notes

- Searching and exact counts scan the whole table, so they grow with its size; everything that can use an index stays around a millisecond.
- Sorting on MySQL used to take 342 ms because the paging tiebreak went the opposite direction of the sort and MySQL couldn't read the index backwards. The tiebreak now follows the sort; it takes 0.9 ms.
