use std::fmt::{self, Display};

use actix_web::http::StatusCode;
use backtrace::Backtrace;
use deadpool::managed::{PoolError, TimeoutType};
use thiserror::Error;

use crate::errors::ReportableError;

#[derive(PartialEq, Eq, Debug)]
pub enum MutateRowStatus {
    OK,
    Cancelled,
    Unknown,
    InvalidArgument,
    DeadlineExceeded,
    NotFound,
    AlreadyExists,
    PermissionDenied,
    ResourceExhausted,
    FailedPrecondition,
    Aborted,
    OutOfRange,
    Unimplemented,
    Internal,
    Unavailable,
    DataLoss,
    Unauthenticated,
}

impl MutateRowStatus {
    pub fn is_ok(&self) -> bool {
        self == &Self::OK
    }
}

impl From<i32> for MutateRowStatus {
    fn from(v: i32) -> Self {
        match v {
            0 => Self::OK,
            1 => Self::Cancelled,
            2 => Self::Unknown,
            3 => Self::InvalidArgument,
            4 => Self::DeadlineExceeded,
            5 => Self::NotFound,
            6 => Self::AlreadyExists,
            7 => Self::PermissionDenied,
            8 => Self::ResourceExhausted,
            9 => Self::FailedPrecondition,
            10 => Self::Aborted,
            11 => Self::OutOfRange,
            12 => Self::Unimplemented,
            13 => Self::Internal,
            14 => Self::Unavailable,
            15 => Self::DataLoss,
            16 => Self::Unauthenticated,
            _ => Self::Unknown,
        }
    }
}

impl Display for MutateRowStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            MutateRowStatus::OK => "Ok",
            MutateRowStatus::Cancelled => "Cancelled",
            MutateRowStatus::Unknown => "Unknown",
            MutateRowStatus::InvalidArgument => "Invalid Argument",
            MutateRowStatus::DeadlineExceeded => "Deadline Exceeded",
            MutateRowStatus::NotFound => "Not Found",
            MutateRowStatus::AlreadyExists => "Already Exists",
            MutateRowStatus::PermissionDenied => "Permission Denied",
            MutateRowStatus::ResourceExhausted => "Resource Exhausted",
            MutateRowStatus::FailedPrecondition => "Failed Precondition",
            MutateRowStatus::Aborted => "Aborted",
            MutateRowStatus::OutOfRange => "Out of Range",
            MutateRowStatus::Unimplemented => "Unimplemented",
            MutateRowStatus::Internal => "Internal",
            MutateRowStatus::Unavailable => "Unavailable",
            MutateRowStatus::DataLoss => "Data Loss",
            MutateRowStatus::Unauthenticated => "Unauthenticated",
        })
    }
}

impl MutateRowStatus {
    pub fn status(&self) -> StatusCode {
        match self {
            MutateRowStatus::OK => StatusCode::OK,
            // Some of these were taken from the java-bigtable-hbase retry handlers
            MutateRowStatus::Aborted
            | MutateRowStatus::DeadlineExceeded
            | MutateRowStatus::Internal
            | MutateRowStatus::ResourceExhausted
            | MutateRowStatus::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[derive(Debug, Error)]
pub enum BigTableError {
    #[error("Invalid Row Response: {0}")]
    InvalidRowResponse(#[source] grpcio::Error),

    #[error("Invalid Chunk")]
    InvalidChunk(String),

    #[error("BigTable read error: {0}")]
    Read(#[source] grpcio::Error),

    #[error("BigTable write timestamp error: {0}")]
    WriteTime(#[source] std::time::SystemTimeError),

    #[error("Bigtable write error: {0}")]
    Write(#[source] grpcio::Error),

    #[error("GRPC Error: {0}")]
    GRPC(#[source] grpcio::Error),

    /// Return a GRPC status code and any message.
    /// See https://grpc.github.io/grpc/core/md_doc_statuscodes.html
    #[error("Bigtable status response: {0:?}")]
    Status(MutateRowStatus, String),

    #[error("BigTable Admin Error: {0}")]
    Admin(String, Option<String>),

    /// General Pool errors
    #[error("Pool Error: {0}")]
    Pool(Box<PoolError<BigTableError>>),

    /// Timeout occurred while getting a pooled connection
    #[error("Pool Timeout: {0:?}")]
    PoolTimeout(TimeoutType),

    #[error("BigTable config error: {0}")]
    Config(String),
}

impl BigTableError {
    pub fn status(&self) -> StatusCode {
        match self {
            BigTableError::PoolTimeout(_) => StatusCode::SERVICE_UNAVAILABLE,
            BigTableError::Status(e, _) => e.status(),
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl ReportableError for BigTableError {
    fn reportable_source(&self) -> Option<&(dyn ReportableError + 'static)> {
        None
    }

    fn backtrace(&self) -> Option<&Backtrace> {
        None
    }

    fn is_sentry_event(&self) -> bool {
        #[allow(clippy::match_like_matches_macro)]
        match self {
            BigTableError::PoolTimeout(_) => false,
            _ => true,
        }
    }

    fn metric_label(&self) -> Option<&'static str> {
        let err = match self {
            BigTableError::InvalidRowResponse(_) => "storage.bigtable.error.invalid_row_response",
            BigTableError::InvalidChunk(_) => "storage.bigtable.error.invalid_chunk",
            BigTableError::Read(_) => "storage.bigtable.error.read",
            BigTableError::Write(_) => "storage.bigtable.error.write",
            BigTableError::Status(_, _) => "storage.bigtable.error.status",
            BigTableError::WriteTime(_) => "storage.bigtable.error.writetime",
            BigTableError::Admin(_, _) => "storage.bigtable.error.admin",
            BigTableError::Pool(_) => "storage.bigtable.error.pool",
            BigTableError::PoolTimeout(_) => "storage.bigtable.error.pool_timeout",
            BigTableError::GRPC(_) => "storage.bigtable.error.grpc",
            BigTableError::Config(_) => "storage.bigtable.error.config",
        };
        Some(err)
    }

    fn tags(&self) -> Vec<(&str, String)> {
        #[allow(clippy::match_like_matches_macro)]
        match self {
            BigTableError::PoolTimeout(tt) => vec![("type", format!("{tt:?}").to_lowercase())],
            _ => vec![],
        }
    }

    fn extras(&self) -> Vec<(&str, String)> {
        match self {
            BigTableError::InvalidRowResponse(s) => vec![("error", s.to_string())],
            BigTableError::InvalidChunk(s) => vec![("error", s.to_string())],
            BigTableError::GRPC(s) => vec![("error", s.to_string())],
            BigTableError::Read(s) => vec![("error", s.to_string())],
            BigTableError::Write(s) => vec![("error", s.to_string())],
            BigTableError::Status(code, s) => {
                vec![("code", code.to_string()), ("error", s.to_string())]
            }
            BigTableError::WriteTime(s) => vec![("error", s.to_string())],
            BigTableError::Admin(s, raw) => {
                let mut x = vec![("error", s.to_owned())];
                if let Some(raw) = raw {
                    x.push(("raw", raw.to_string()));
                };
                x
            }
            BigTableError::Pool(e) => vec![("error", e.to_string())],
            _ => vec![],
        }
    }
}
