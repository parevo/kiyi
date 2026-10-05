//! Shared streaming loop that turns sqlx's `fetch_many` output into batched `QueryEvent`s.
//!
//! Written as a macro instead of generic functions: sqlx's `Executor` impls don't satisfy
//! the higher-ranked `Send` bounds `async_trait` needs when the database type is generic.
//! Each driver invokes `define_engine!` in its own module, next to its `column`, `cell`
//! and `rows_affected` functions.

use std::time::Duration;

use crate::types::{ColumnMeta, QueryEvent, Sink};

/// Rows are flushed to the UI at least this often, so the first rows appear immediately.
pub(crate) const FLUSH_INTERVAL: Duration = Duration::from_millis(40);
pub(crate) const MAX_BATCH: usize = 2_000;

pub(crate) struct StreamOutcome {
    pub statements: u32,
    /// Rows-affected of the final statement when it produced no rows; the caller may
    /// still want to describe its columns (an empty SELECT) before reporting it.
    pub trailing_empty: Option<u64>,
}

macro_rules! define_engine {
    ($conn:ty) => {
        use $crate::drivers::engine::{StreamOutcome, FLUSH_INTERVAL, MAX_BATCH};

        async fn stream(conn: &mut $conn, sql: &str, sink: &Sink<'_>) -> Result<StreamOutcome> {
            use futures::TryStreamExt;
            use sqlx::Either;
            use std::time::Instant;

            let mut results = sqlx::Executor::fetch_many(&mut *conn, sqlx::raw_sql(sql));
            let mut kinds: Option<Vec<ValueKind>> = None;
            let mut batch: Vec<Vec<Cell>> = Vec::new();
            let mut last_flush = Instant::now();
            let mut statements = 0u32;
            let mut pending_empty: Option<u64> = None;

            let flush = |batch: &mut Vec<Vec<Cell>>| {
                if !batch.is_empty() {
                    sink($crate::types::QueryEvent::Rows { rows: std::mem::take(batch) });
                }
            };

            while let Some(item) = results.try_next().await? {
                if let Some(affected) = pending_empty.take() {
                    sink($crate::types::QueryEvent::StatementDone { rows_affected: affected });
                }
                match item {
                    Either::Right(row) => {
                        if kinds.is_none() {
                            let columns: Vec<ColumnMeta> = row.columns().iter().map(column).collect();
                            kinds = Some(columns.iter().map(|c| c.kind).collect());
                            sink($crate::types::QueryEvent::Columns { columns });
                            last_flush = Instant::now() - FLUSH_INTERVAL;
                        }
                        let k = kinds.as_ref().unwrap();
                        batch.push((0..row.len()).map(|i| cell(&row, i, k[i])).collect());
                        if batch.len() >= MAX_BATCH || last_flush.elapsed() >= FLUSH_INTERVAL {
                            flush(&mut batch);
                            last_flush = Instant::now();
                        }
                    }
                    Either::Left(result) => {
                        flush(&mut batch);
                        statements += 1;
                        let affected = rows_affected(&result);
                        if kinds.take().is_none() {
                            pending_empty = Some(affected);
                        } else {
                            sink($crate::types::QueryEvent::StatementDone { rows_affected: affected });
                        }
                    }
                }
            }
            flush(&mut batch);
            Ok(StreamOutcome { statements, trailing_empty: pending_empty })
        }

        /// Columns of a single statement without running it, for SELECTs that returned no rows.
        async fn describe_columns(conn: &mut $conn, sql: &str) -> Vec<ColumnMeta> {
            match sqlx::Executor::describe(&mut *conn, sql).await {
                Ok(d) => d.columns().iter().map(column).collect(),
                Err(_) => Vec::new(),
            }
        }

        /// Runs a catalog query and returns every value as text.
        async fn text_rows(conn: &mut $conn, sql: &str) -> Result<Vec<Vec<Cell>>> {
            let rows = sqlx::Executor::fetch_all(&mut *conn, sqlx::raw_sql(sql)).await?;
            Ok(rows.iter().map(|row| (0..row.len()).map(|i| cell(row, i, ValueKind::Text)).collect()).collect())
        }

        fn first_cell(rows: Vec<Vec<Cell>>) -> Option<String> {
            rows.into_iter().next().and_then(|r| r.into_iter().next().flatten())
        }

        async fn fetch_all(conn: &mut $conn, sql: &str) -> Result<(Vec<ColumnMeta>, Vec<Vec<Cell>>)> {
            let collected = std::sync::Mutex::new((Vec::new(), Vec::new()));
            let sink = |e: $crate::types::QueryEvent| {
                let mut c = collected.lock().unwrap();
                match e {
                    $crate::types::QueryEvent::Columns { columns } => {
                        c.0 = columns;
                        c.1.clear();
                    }
                    $crate::types::QueryEvent::Rows { rows } => c.1.extend(rows),
                    _ => {}
                }
            };
            let outcome = stream(conn, sql, &sink).await?;
            if outcome.trailing_empty.is_some() && collected.lock().unwrap().0.is_empty() {
                collected.lock().unwrap().0 = describe_columns(conn, sql).await;
            }
            Ok(collected.into_inner().unwrap())
        }

        async fn script(
            conn: &mut $conn,
            begin: &str,
            statements: &[String],
            transactional: bool,
            expect_single_row: bool,
        ) -> Result<Vec<u64>> {
            use $crate::error::Error;
            async fn rollback(conn: &mut $conn) {
                let _ = sqlx::Executor::execute(&mut *conn, sqlx::raw_sql("ROLLBACK")).await;
            }
            if transactional {
                sqlx::Executor::execute(&mut *conn, sqlx::raw_sql(begin)).await?;
            }
            let mut affected = Vec::with_capacity(statements.len());
            for (index, statement) in statements.iter().enumerate() {
                let failure = match sqlx::Executor::execute(&mut *conn, sqlx::raw_sql(statement)).await {
                    Ok(r) if expect_single_row && rows_affected(&r) != 1 => Some(Error::RowMismatch(rows_affected(&r))),
                    Ok(r) => {
                        affected.push(rows_affected(&r));
                        None
                    }
                    Err(e) => Some(e.into()),
                };
                if let Some(source) = failure {
                    if transactional {
                        rollback(conn).await;
                    }
                    return Err(Error::Script { index, source: Box::new(source) });
                }
            }
            if transactional {
                sqlx::Executor::execute(&mut *conn, sqlx::raw_sql("COMMIT")).await?;
            }
            Ok(affected)
        }

        /// Session id lookup, streaming, and the empty-SELECT header fallback.
        async fn run(conn: &mut $conn, session_sql: &str, sql: &str, on_session: &(dyn Fn(u64) + Send + Sync), sink: &Sink<'_>) -> Result<()> {
            if let Some(id) = first_cell(text_rows(conn, session_sql).await?).and_then(|v| v.parse().ok()) {
                on_session(id);
            }
            let outcome = stream(conn, sql, sink).await?;
            let columns = match outcome.trailing_empty {
                Some(_) if outcome.statements == 1 => Some(describe_columns(conn, sql).await),
                _ => None,
            };
            $crate::drivers::engine::finish_trailing(&outcome, columns, sink);
            Ok(())
        }
    };
}
pub(crate) use define_engine;

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(2 + bytes.len() * 2);
    out.push_str("0x");
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Reports a trailing empty statement, emitting its described columns first so an
/// empty SELECT still shows its headers.
pub(crate) fn finish_trailing(outcome: &StreamOutcome, columns: Option<Vec<ColumnMeta>>, sink: &Sink<'_>) {
    if let Some(affected) = outcome.trailing_empty {
        if let Some(columns) = columns.filter(|c| !c.is_empty()) {
            sink(QueryEvent::Columns { columns });
        }
        sink(QueryEvent::StatementDone { rows_affected: affected });
    }
}
