//! This DynamoDB client is a selectively upgraded version of `DynamoStorage` in `autopush_common`.
//! Due to #172, autoendpoint cannot use any Tokio 0.1 code, so for now we have to copy and update
//! pieces of `DynamoStorage` as needed.

// pub mod client;
pub mod error;
// mod retry;

// #[cfg(test)]
// pub use autopush_common::db::mock::MockDbClient;
//pub mod mock;
