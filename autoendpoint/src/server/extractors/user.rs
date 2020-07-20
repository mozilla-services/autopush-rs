//! User validations

use crate::error::{ApiErrorKind, ApiResult};
use crate::server::extractors::routers::RouterType;
use crate::server::ServerState;
use autopush_common::db::{DynamoDbUser, DynamoStorage};
use cadence::{Counted, StatsdClient};
use futures::compat::Future01CompatExt;
use uuid::Uuid;

/// Perform some validations on the user, including:
/// - Validate router type
/// - (WebPush) Check that the subscription/channel exists
/// - (WebPush) Drop user if inactive
///
/// Returns an enum representing the user's router type.
pub async fn validate_user(
    user: &DynamoDbUser,
    channel_id: &Uuid,
    state: &ServerState,
) -> ApiResult<RouterType> {
    let router_type = match user.router_type.parse::<RouterType>() {
        Ok(router_type) => router_type,
        Err(_) => {
            debug!("Unknown router type, dropping user"; "user" => ?user);
            drop_user(&user.uaid, &state.ddb, &state.metrics).await?;
            return Err(ApiErrorKind::NoSubscription.into());
        }
    };

    if router_type == RouterType::WebPush {
        validate_webpush_user(user, channel_id, &state.ddb, &state.metrics).await?;
    }

    Ok(router_type)
}

/// Make sure the user is not inactive and the subscription channel exists
async fn validate_webpush_user(
    user: &DynamoDbUser,
    channel_id: &Uuid,
    ddb: &DynamoStorage,
    metrics: &StatsdClient,
) -> ApiResult<()> {
    // Make sure the user is active (has a valid message table)
    let message_table = match user.current_month.as_ref() {
        Some(table) => table,
        None => {
            debug!("Missing `current_month` value, dropping user"; "user" => ?user);
            drop_user(&user.uaid, ddb, metrics).await?;
            return Err(ApiErrorKind::NoSubscription.into());
        }
    };

    if !ddb.message_table_names.contains(message_table) {
        debug!("User is inactive, dropping user"; "user" => ?user);
        drop_user(&user.uaid, ddb, metrics).await?;
        return Err(ApiErrorKind::NoSubscription.into());
    }

    // Make sure the subscription channel exists
    let channel_ids = ddb
        .get_user_channels(&user.uaid, message_table)
        .compat()
        .await
        .map_err(ApiErrorKind::Database)?;

    if !channel_ids.contains(channel_id) {
        return Err(ApiErrorKind::NoSubscription.into());
    }

    Ok(())
}

/// Drop a user and increment associated metric
async fn drop_user(uaid: &Uuid, ddb: &DynamoStorage, metrics: &StatsdClient) -> ApiResult<()> {
    metrics
        .incr_with_tags("updates.drop_user")
        .with_tag("errno", "102")
        .send();

    ddb.drop_uaid(uaid)
        .compat()
        .await
        .map_err(ApiErrorKind::Database)?;

    Ok(())
}
