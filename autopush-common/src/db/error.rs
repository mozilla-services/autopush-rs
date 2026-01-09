use actix_web::http::StatusCode;
use serde_json::Error as Serde_Error;

use thiserror::Error;

#[cfg(feature = "bigtable")]
use crate::db::bigtable::BigTableError;
use crate::errors::ReportableError;

pub type DbResult<T> = Result<T, DbError>;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("Error while performing (de)serialization: {0}")]
    Serialization(String),

    #[error("Error deserializing to u64: {0}")]
    DeserializeU64(String),

    #[error("Error deserializing to String: {0}")]
    DeserializeString(String),

    #[error("Unable to determine table status")]
    TableStatusUnknown,

    #[cfg(feature = "bigtable")]
    #[error("BigTable error: {0}")]
    BTError(#[from] BigTableError),

    #[cfg(feature = "redis")]
    #[error("Redis error {0}")]
    RedisError(#[from] redis::RedisError),

    #[error("Serde Json Parse Error {0}")]
    SerdeError(#[from] Serde_Error),

    #[error("Connection failure: {0}")]
    ConnectionError(String),

    #[error("The conditional request failed")]
    Conditional,

    #[error("Database integrity error: {}", _0)]
    Integrity(String, Option<String>),

    #[error("Unknown Database Error: {0}")]
    General(String),

    // Return a 503 error
    #[error("Process pending, please wait.")]
    Backoff(String),

    #[cfg(feature = "postgres")]
    #[error("Postgres Error: {0}")]
    PgError(#[from] tokio_postgres::Error),
}

impl DbError {
    pub fn status(&self) -> StatusCode {
        match self {
            #[cfg(feature = "bigtable")]
            Self::BTError(e) => e.status(),
            Self::Backoff(_) => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl ReportableError for DbError {
    fn reportable_source(&self) -> Option<&(dyn ReportableError + 'static)> {
        match &self {
            #[cfg(feature = "bigtable")]
            DbError::BTError(e) => Some(e),
            _ => None,
        }
    }

    fn is_sentry_event(&self) -> bool {
        match &self {
            #[cfg(feature = "bigtable")]
            DbError::BTError(e) => e.is_sentry_event(),
            _ => false,
        }
    }

    fn metric_label(&self) -> Option<&'static str> {
        match &self {
            #[cfg(feature = "bigtable")]
            DbError::BTError(e) => e.metric_label(),
            DbError::Backoff(_) => Some("storage.error.backoff"),
            _ => None,
        }
    }

    fn extras(&self) -> Vec<(&str, String)> {
        match &self {
            #[cfg(feature = "bigtable")]
            DbError::BTError(e) => e.extras(),
            DbError::Backoff(e) => {
                vec![("raw", e.to_string())]
            }
            DbError::Integrity(_, Some(row)) => vec![("row", row.clone())],
            _ => vec![],
        }
    }
}
