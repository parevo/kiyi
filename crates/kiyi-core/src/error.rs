use serde::Serialize;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Db(#[from] sqlx::Error),
    #[error("Connection not found: {0}")]
    UnknownConnection(String),
    #[error("Not connected")]
    NotConnected,
    #[error("Invalid connection address: {0}")]
    InvalidUrl(String),
    #[error("Timed out: {0}")]
    Timeout(&'static str),
    #[error("Keychain error: {0}")]
    Secret(#[from] keyring::Error),
    #[error("File error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Data error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
    /// A statement in a multi-statement script failed; everything was rolled back.
    #[error("{source}")]
    Script { index: usize, source: Box<Error> },
    #[error("Expected to change 1 row but {0} were affected. Someone may have changed or deleted it in the meantime.")]
    RowMismatch(u64),
}

/// What the UI receives: a human message plus an optional machine code
/// (SQLSTATE / MySQL error number) and the 1-based character position of the error.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorInfo {
    pub message: String,
    pub code: Option<String>,
    pub position: Option<u32>,
    /// For scripts: which statement failed.
    pub statement_index: Option<usize>,
}

impl From<&Error> for ErrorInfo {
    fn from(err: &Error) -> Self {
        match err {
            Error::Db(sqlx::Error::Database(db)) => {
                let position = db
                    .try_downcast_ref::<sqlx::postgres::PgDatabaseError>()
                    .and_then(|pg| match pg.position() {
                        Some(sqlx::postgres::PgErrorPosition::Original(p)) => Some(p as u32),
                        _ => None,
                    });
                ErrorInfo {
                    message: db.message().to_string(),
                    code: db.code().map(|c| c.into_owned()),
                    position,
                    statement_index: None,
                }
            }
            Error::Script { index, source } => ErrorInfo { statement_index: Some(*index), ..ErrorInfo::from(&**source) },
            other => ErrorInfo { message: other.to_string(), code: None, position: None, statement_index: None },
        }
    }
}

impl From<Error> for ErrorInfo {
    fn from(err: Error) -> Self {
        ErrorInfo::from(&err)
    }
}

/// A short, actionable explanation for connection failures, shown in the connection test.
pub fn explain_connect_error(err: &Error) -> String {
    match err {
        Error::Db(sqlx::Error::Database(db)) => match db.code().as_deref() {
            Some("28P01") | Some("28000") | Some("1045") => "The username or password is incorrect.".into(),
            Some("3D000") | Some("1049") => "That database does not exist. Check its name.".into(),
            Some("53300") | Some("1040") => "The server has reached its connection limit.".into(),
            Some("14") => "Couldn't open the database file. Check that it exists and you can read it.".into(),
            Some("26") => "That file isn't a SQLite database.".into(),
            _ => db.message().to_string(),
        },
        Error::Db(sqlx::Error::Io(io)) => {
            let text = io.to_string();
            match io.kind() {
                std::io::ErrorKind::ConnectionRefused => {
                    "Connection refused. Check the host and port, and that the server is running.".into()
                }
                std::io::ErrorKind::NotFound => {
                    "No database socket at that path. Check the folder (PostgreSQL often uses /tmp or /var/run/postgresql) and that the server runs on this computer.".into()
                }
                std::io::ErrorKind::TimedOut => "The server did not respond. A firewall or security group may be blocking it.".into(),
                _ if text.contains("lookup") || text.contains("nodename") => {
                    "The host name could not be resolved. Check the address.".into()
                }
                _ => text,
            }
        }
        Error::Db(sqlx::Error::Tls(e)) => format!("TLS/SSL error: {e}"),
        Error::Db(sqlx::Error::PoolTimedOut) | Error::Timeout(_) => {
            "The server did not respond in time. Check network access (VPN, firewall, security group).".into()
        }
        other => other.to_string(),
    }
}

/// "No such table" from any of the databases (e.g. dropped by someone else a moment ago).
pub(crate) fn is_missing_table(e: &Error) -> bool {
    match e {
        Error::Db(sqlx::Error::Database(db)) => matches!(db.code().as_deref(), Some("42P01") | Some("1146")) || db.message().contains("no such table"),
        _ => false,
    }
}

/// True when the error is the server confirming a user-requested cancel.
pub(crate) fn is_cancel_error(err: &Error) -> bool {
    matches!(err, Error::Db(sqlx::Error::Database(db)) if matches!(db.code().as_deref(), Some("57014") | Some("1317")))
}
