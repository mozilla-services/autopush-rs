use backtrace::Backtrace;
use thiserror::Error;

use crate::errors::ReportableError;

#[derive(Debug, Error)]
pub enum BigTableError {
    #[error("Invalid Row Response")]
    InvalidRowResponse(String),

    #[error("Invalid Chunk")]
    InvalidChunk(String),

    #[error("BigTable read error")]
    Read(String),

    #[error("BigTable write error")]
    Write(String),

    #[error("BigTable Admin Error")]
    Admin(String, Option<String>),

    #[error("Bigtable Recycle request")]
    Recycle,

    /// General Pool builder errors.
    #[error("Pool Error")]
    Pool(String),
}

impl ReportableError for BigTableError {
    fn reportable_source(&self) -> Option<&(dyn ReportableError + 'static)> {
        None
    }

    fn backtrace(&self) -> Option<&Backtrace> {
        None
    }

    fn is_sentry_event(&self) -> bool {
        // eventually, only capture important errors
        //matches!(&self, BigTableError::Admin(_, _))
        true
    }

    fn metric_label(&self) -> Option<&'static str> {
        let err = match &self {
            BigTableError::InvalidRowResponse(_) => "storage.bigtable.error.invalid_row_response",
            BigTableError::InvalidChunk(_) => "storage.bigtable.error.invalid_chunk",
            BigTableError::Read(_) => "storage.bigtable.error.read",
            BigTableError::Write(_) => "storage.bigtable.error.write",
            BigTableError::Admin(_, _) => "storage.bigtable.error.admin",
            BigTableError::Recycle => "storage.bigtable.error.recycle",
            BigTableError::Pool(_) => "storage.bigtable.error.pool",
        };
        Some(err)
    }

    fn extras(&self) -> Vec<(&str, String)> {
        match &self {
            BigTableError::InvalidRowResponse(s) => vec![("error", s.to_owned())],
            BigTableError::InvalidChunk(s) => vec![("error", s.to_owned())],
            BigTableError::Read(s) => vec![("error", s.to_owned())],
            BigTableError::Write(s) => vec![("error", s.to_owned())],
            BigTableError::Admin(s, raw) => {
                let mut x = vec![("error", s.to_owned())];
                if let Some(raw) = raw {
                    x.push(("raw", raw.to_owned()));
                };
                x
            }
            BigTableError::Pool(s) => vec![("error", s.to_owned())],
            _ => vec![],
        }
    }
}
