use serde::Serialize;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Db(#[from] sqlx::Error),
    #[error("Bağlantı bulunamadı: {0}")]
    UnknownConnection(String),
    #[error("Bağlantı açık değil")]
    NotConnected,
    #[error("Geçersiz bağlantı adresi: {0}")]
    InvalidUrl(String),
    #[error("Zaman aşımı: {0}")]
    Timeout(&'static str),
    #[error("Keychain hatası: {0}")]
    Secret(#[from] keyring::Error),
    #[error("Dosya hatası: {0}")]
    Io(#[from] std::io::Error),
    #[error("Veri hatası: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
    /// A statement in a multi-statement script failed; everything was rolled back.
    #[error("{source}")]
    Script { index: usize, source: Box<Error> },
    #[error("Beklenen 1 satır yerine {0} satır etkilendi. Satır başka biri tarafından değiştirilmiş ya da silinmiş olabilir.")]
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
            Some("28P01") | Some("28000") | Some("1045") => "Kullanıcı adı veya şifre yanlış.".into(),
            Some("3D000") | Some("1049") => "Veritabanı bulunamadı. Adını kontrol et.".into(),
            Some("53300") | Some("1040") => "Sunucu bağlantı limitine ulaşmış.".into(),
            _ => db.message().to_string(),
        },
        Error::Db(sqlx::Error::Io(io)) => {
            let text = io.to_string();
            match io.kind() {
                std::io::ErrorKind::ConnectionRefused => {
                    "Bağlantı reddedildi. Host ve port doğru mu, sunucu çalışıyor mu?".into()
                }
                std::io::ErrorKind::TimedOut => "Sunucu yanıt vermedi. Firewall ya da security group engelliyor olabilir.".into(),
                _ if text.contains("lookup") || text.contains("nodename") => {
                    "Host adı çözümlenemedi. Adresi kontrol et.".into()
                }
                _ => text,
            }
        }
        Error::Db(sqlx::Error::Tls(e)) => format!("TLS/SSL hatası: {e}"),
        Error::Db(sqlx::Error::PoolTimedOut) | Error::Timeout(_) => {
            "Sunucu zamanında yanıt vermedi. Ağ erişimini (VPN, firewall, security group) kontrol et.".into()
        }
        other => other.to_string(),
    }
}

/// True when the error is the server confirming a user-requested cancel.
pub(crate) fn is_cancel_error(err: &Error) -> bool {
    matches!(err, Error::Db(sqlx::Error::Database(db)) if matches!(db.code().as_deref(), Some("57014") | Some("1317")))
}
