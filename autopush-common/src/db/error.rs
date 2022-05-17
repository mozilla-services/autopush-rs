use thiserror::Error;

use rusoto_core::RusotoError;
use rusoto_dynamodb::{
    DeleteItemError, DescribeTableError, GetItemError, PutItemError, UpdateItemError, QueryError,
};
use tokio_postgres::Error as PgError;

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

    #[error("Database error while performing DescribeTable")]
    DdbDescribeTable(#[from] RusotoError<DescribeTableError>),

    #[error("Database error while performing Query")]
    DdbQuery(#[from] RusotoError<QueryError>),

    #[error("Error while performing (de)serialization: {0}")]
    Serialization(#[from] serde_dynamodb::Error),

    #[error("Unable to determine table status")]
    TableStatusUnknown,

    #[error("Postgres error")]
    PgError(#[from] PgError),

    #[error("Connection failure {0}")]
    ConnectionError(String),

    #[error("Unknown Database Error {0}")]
    General(String),
}
