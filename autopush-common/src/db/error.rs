use backtrace::Backtrace;
use rusoto_core::RusotoError;
use rusoto_dynamodb::{
    BatchWriteItemError, DeleteItemError, DescribeTableError, GetItemError, PutItemError,
    QueryError, UpdateItemError,
};
use thiserror::Error;

#[cfg(feature = "bigtable")]
use crate::db::bigtable::BigTableError;
use crate::errors::ReportableError;

pub type DbResult<T> = Result<T, DbError>;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("Database error while performing GetItem")]
    DdbGetItem(#[from] RusotoError<GetItemError>),

    #[error("Database error while performing UpdateItem")]
    DdbUpdateItem(#[from] RusotoError<UpdateItemError>),

    #[error("Database error while performing PutItem")]
    DdbPutItem(#[from] RusotoError<PutItemError>),

    #[error("Database error while performing DeleteItem")]
    DdbDeleteItem(#[from] RusotoError<DeleteItemError>),

    #[error("Database error while performing BatchWriteItem")]
    DdbBatchWriteItem(#[from] RusotoError<BatchWriteItemError>),

    #[error("Database error while performing DescribeTable")]
    DdbDescribeTable(#[from] RusotoError<DescribeTableError>),

    #[error("Database error while performing Query")]
    DdbQuery(#[from] RusotoError<QueryError>),

    #[error("Error while performing DynamoDB (de)serialization: {0}")]
    DdbSerialization(#[from] serde_dynamodb::Error),

    #[error("Error while performing (de)serialization: {0}")]
    Serialization(String),

    #[error("Error deserializing to u64: {0}")]
    DeserializeU64(String),

    #[error("Error deserializing to String: {0}")]
    DeserializeString(String),

    #[error("Unable to determine table status")]
    TableStatusUnknown,

    #[cfg(feature = "bigtable")]
    #[error("BigTable error {0}")]
    BTError(#[from] BigTableError),

    /*
    #[error("Postgres error")]
    PgError(#[from] PgError),
    */
    #[error("Connection failure {0}")]
    ConnectionError(String),

    #[error("Unknown Database Error {0}")]
    General(String),
}

impl ReportableError for DbError {
    fn reportable_source(&self) -> Option<&(dyn ReportableError + 'static)> {
        match &self {
            #[cfg(feature = "bigtable")]
            DbError::BTError(e) => Some(e),
            _ => None,
        }
    }

    fn backtrace(&self) -> Option<&Backtrace> {
        None
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
            _ => None,
        }
    }

    fn extras(&self) -> Vec<(&str, String)> {
        match &self {
            #[cfg(feature = "bigtable")]
            DbError::BTError(e) => e.extras(),
            _ => vec![],
        }
    }
}
