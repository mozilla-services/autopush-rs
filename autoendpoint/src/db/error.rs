use thiserror::Error;

use rusoto_core::RusotoError;
use rusoto_dynamodb::{
    DeleteItemError, DescribeTableError, GetItemError, PutItemError, UpdateItemError,
};

pub type DbResult<T> = Result<T, DbError>;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("Database error while performing GetItem")]
    GetItem(#[from] RusotoError<GetItemError>),

    #[error("Database error while performing UpdateItem")]
    UpdateItem(#[from] RusotoError<UpdateItemError>),

    #[error("Database error while performing PutItem")]
    PutItem(#[from] RusotoError<PutItemError>),

    #[error("Database error while performing DeleteItem")]
    DeleteItem(#[from] RusotoError<DeleteItemError>),

    #[error("Database error while performing DescribeTable")]
    DescribeTable(#[from] RusotoError<DescribeTableError>),

    #[error("Error while performing (de)serialization: {0}")]
    Serialization(#[from] serde_dynamodb::Error),

    #[error("Unable to determine table status")]
    TableStatusUnknown,

    #[error("Could not connect to database: {0}")]
    Connection(String),

    #[error("Postgres Error: {0}")]
    Postgres(#[from] tokio_postgres::error::Error),

    #[error("General Error: {0}")]
    General(String),
}
