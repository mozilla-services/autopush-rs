use again::RetryPolicy;
use cadence::{CountedExt, StatsdClient};
use rusoto_core::RusotoError;
use rusoto_dynamodb::{
    DeleteItemError, DescribeTableError, GetItemError, PutItemError, UpdateItemError,
};
use std::sync::Arc;

/// Create a retry function for the given error
macro_rules! retryable_error {
    ($name:ident, $error:tt, $error_tag:expr) => {
        pub fn $name(metrics: Arc<StatsdClient>) -> impl Fn(&RusotoError<$error>) -> bool {
            move |err| match err {
                RusotoError::Service($error::InternalServerError(_))
                | RusotoError::Service($error::ProvisionedThroughputExceeded(_)) => {
                    metrics
                        .incr_with_tags("database.retry")
                        .with_tag("error", $error_tag)
                        .send();
                    true
                }
                _ => false,
            }
        }
    };
}

retryable_error!(retryable_getitem_error, GetItemError, "get_item");
retryable_error!(retryable_updateitem_error, UpdateItemError, "update_item");
retryable_error!(retryable_putitem_error, PutItemError, "put_item");
retryable_error!(retryable_delete_error, DeleteItemError, "delete_item");

// DescribeTableError does not have a ProvisionedThroughputExceeded variant
pub fn retryable_describe_table_error(
    metrics: Arc<StatsdClient>,
) -> impl Fn(&RusotoError<DescribeTableError>) -> bool {
    move |err| match err {
        RusotoError::Service(DescribeTableError::InternalServerError(_)) => {
            metrics
                .incr_with_tags("database.retry")
                .with_tag("error", "describe_table_error")
                .send();
            true
        }
        _ => false,
    }
}

/// Build an exponential retry policy
pub fn retry_policy() -> RetryPolicy {
    RetryPolicy::default().with_jitter(true)
}
