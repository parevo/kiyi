mod engine;
mod mysql;
mod postgres;

use std::sync::Arc;

use async_trait::async_trait;

use crate::config::{ConnectionConfig, DbKind};
use crate::design::TableDetails;
use crate::dialect::Dialect;
use crate::error::Result;
use crate::types::{Cell, ColumnMeta, SchemaSnapshot, Sink};

/// One live connection pool to a database server.
#[async_trait]
pub trait DbDriver: Send + Sync {
    async fn server_version(&self) -> Result<String>;

    async fn schema(&self) -> Result<SchemaSnapshot>;

    /// Runs `sql` (one or more statements) and streams results into `sink`.
    /// `on_session` receives the server-side session id before execution starts,
    /// so the caller can cancel the query from another connection.
    async fn execute(&self, sql: &str, on_session: &(dyn Fn(u64) + Send + Sync), sink: &Sink<'_>) -> Result<()>;

    /// Asks the server to cancel whatever `session_id` is currently running.
    async fn cancel(&self, session_id: u64) -> Result<()>;

    async fn close(&self);

    /// Quoting rules for this server and session settings.
    fn dialect(&self) -> Dialect;

    /// Columns, primary key, indexes and foreign keys of one table or view.
    async fn table_details(&self, schema: Option<&str>, table: &str) -> Result<TableDetails>;

    /// Runs a single query and returns the whole result (for small, bounded reads like a page).
    async fn fetch(&self, sql: &str) -> Result<(Vec<ColumnMeta>, Vec<Vec<Cell>>)>;

    /// Runs statements in order, inside one transaction when `transactional`.
    /// With `expect_single_row`, any statement not affecting exactly one row aborts the
    /// script — so a grid edit never silently touches zero or many rows.
    async fn execute_script(&self, statements: &[String], transactional: bool, expect_single_row: bool) -> Result<Vec<u64>>;
}

pub async fn open(config: &ConnectionConfig, password: Option<&str>) -> Result<Arc<dyn DbDriver>> {
    Ok(match config.kind {
        DbKind::Postgres => Arc::new(postgres::PgDriver::connect(config, password).await?),
        DbKind::Mysql => Arc::new(mysql::MySqlDriver::connect(config, password).await?),
    })
}
