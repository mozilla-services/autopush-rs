//! This DynamoDB client is a selectively upgraded version of `DynamoStorage` in `autopush_common`.
//! Due to #172, autoendpoint cannot use any Tokio 0.1 code, so for now we have to copy and update
//! pieces of `DynamoStorage` as needed.

/// TODO: abstract the various databases into DbClients and move them to autopush_common
/// The DbClient here is the core model that should be used to build out the main one in common.
/// see autopush_common::db::client::DbClient;
///

pub mod client;
pub mod dynamodb;
pub mod error;
pub mod postgres;
mod retry;

#[cfg(test)]
pub mod mock;
