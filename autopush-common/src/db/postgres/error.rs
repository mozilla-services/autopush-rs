use std::fmt::{self, Display};

use actix_web::http::StatusCode;
use deadpool::managed::{PoolError, TimeoutType};
use thiserror::Error;

use crate::errors::ReportableError;

#[derive(PartialEq, Eq, Debug)]

#[derive(Debug, Error)]
pub enum PostgresError {
    #[error("Invalid Row Response: {0}")]
    InvalidRowResponse(#[source] grpcio::Error),

    #[error("Postgres write timestamp error: {0}")]
    WriteTime(#[source] std::time::SystemTimeError),

    #[error("Postgres Admin Error: {0}")]
    Admin(String, Option<String>),

    /// General Pool errors
    #[error("Pool Error: {0}")]
    Pool(Box<PoolError<PostgresError>>),

    /// Timeout occurred while getting a pooled connection
    #[error("Pool Timeout: {0:?}")]
    PoolTimeout(TimeoutType),

    #[error("Postgres config error: {0}")]
    Config(String),
}

impl PostgresError {
    pub fn status(&self) -> StatusCode {
        match self {
            PostgresError::PoolTimeout(_) => StatusCode::SERVICE_UNAVAILABLE,
            PostgresError::Status(e, _) => e.status(),
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl ReportableError for PostgresError {
    fn is_sentry_event(&self) -> bool {
        #[allow(clippy::match_like_matches_macro)]
        match self {
            PostgresError::PoolTimeout(_) => false,
            _ => true,
        }
    }

    fn metric_label(&self) -> Option<&'static str> {
        let err = match self {
            PostgresError::InvalidRowResponse(_) => "storage.Postgres.error.invalid_row_response",
            PostgresError::WriteTime(_) => "storage.Postgres.error.writetime",
            PostgresError::Admin(_, _) => "storage.Postgres.error.admin",
            PostgresError::Pool(_) => "storage.Postgres.error.pool",
            PostgresError::PoolTimeout(_) => "storage.Postgres.error.pool_timeout",
            PostgresError::Config(_) => "storage.Postgres.error.config",
        };
        Some(err)
    }

    fn tags(&self) -> Vec<(&str, String)> {
        #[allow(clippy::match_like_matches_macro)]
        match self {
            PostgresError::PoolTimeout(tt) => vec![("type", format!("{tt:?}").to_lowercase())],
            _ => vec![],
        }
    }

    fn extras(&self) -> Vec<(&str, String)> {
        match self {
            PostgresError::InvalidRowResponse(s) => vec![("error", s.to_string())],
            PostgresError::Read(s) => vec![("error", s.to_string())],
            PostgresError::Write(s) => vec![("error", s.to_string())],
            PostgresError::Status(code, s) => {
                vec![("code", code.to_string()), ("error", s.to_string())]
            }
            PostgresError::WriteTime(s) => vec![("error", s.to_string())],
            PostgresError::Admin(s, raw) => {
                let mut x = vec![("error", s.to_owned())];
                if let Some(raw) = raw {
                    x.push(("raw", raw.to_string()));
                };
                x
            }
            PostgresError::Pool(e) => vec![("error", e.to_string())],
            _ => vec![],
        }
    }
}
