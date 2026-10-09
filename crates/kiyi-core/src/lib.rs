//! Kiyi core: database drivers, connection storage and secrets.
//! Deliberately free of any Tauri dependency so the UI layer can be swapped.

pub mod ai;
pub mod ai_sql;
pub mod backup;
pub mod catalog;
pub mod compare;
pub mod config;
pub mod design;
pub mod discover;
pub mod dialect;
pub mod dml;
pub mod drivers;
pub mod error;
pub mod explain;
pub mod graph;
pub mod secrets;
pub mod ssh_config;
pub mod store;
pub mod transfer;
pub mod tunnel;
pub mod types;
pub mod workspace;

pub use error::{Error, Result};
