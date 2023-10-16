use rusoto_core::RusotoError;
use rusoto_dynamodb::{
    BatchWriteItemError, DeleteItemError, DescribeTableError, GetItemError, PutItemError,
    QueryError, UpdateItemError,
};
use thiserror::Error;

#[cfg(feature = "bigtable")]
use crate::db::bigtable::BigTableError;

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

    #[error("Database Routing Error {0}")]
    DbRouting(String),
}
