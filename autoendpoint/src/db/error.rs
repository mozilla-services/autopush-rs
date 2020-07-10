use thiserror::Error;

use rusoto_core::RusotoError;
use rusoto_dynamodb::{DeleteItemError, GetItemError, PutItemError, UpdateItemError};

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

    #[error("Error while deserializing database response: {0}")]
    Deserialize(#[from] serde_dynamodb::Error),
}
