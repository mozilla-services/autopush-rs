use thiserror::Error;

use rusoto_core::RusotoError;
use rusoto_dynamodb::{DeleteItemError, GetItemError, PutItemError, UpdateItemError};

pub type DbResult<T> = Result<T, DbError>;

#[derive(Debug, Error)]
pub enum DbError {
    #[error(transparent)]
    GetItem(#[from] RusotoError<GetItemError>),

    #[error(transparent)]
    UpdateItem(#[from] RusotoError<UpdateItemError>),

    #[error(transparent)]
    PutItem(#[from] RusotoError<PutItemError>),

    #[error(transparent)]
    DeleteItem(#[from] RusotoError<DeleteItemError>),

    #[error("Error while deserializing database response: {0}")]
    Deserialize(#[from] serde_dynamodb::Error),
}
