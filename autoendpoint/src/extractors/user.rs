//! User validations

use crate::error::{ApiErrorKind, ApiResult};
use crate::extractors::routers::RouterType;
use crate::server::AppState;
use autopush_common::db::{client::DbClient, User};
use cadence::{CountedExt, StatsdClient};
use uuid::Uuid;

/// Perform some validations on the user, including:
/// - Validate router type
/// - (WebPush) Check that the subscription/channel exists
/// - (WebPush) Drop user if inactive
///
/// Returns an enum representing the user's router type.
pub async fn validate_user(
    user: &User,
    channel_id: &Uuid,
    app_state: &AppState,
) -> ApiResult<RouterType> {
    let router_type = match user.router_type.parse::<RouterType>() {
        Ok(router_type) => router_type,
        Err(_) => {
            debug!("Unknown router type, dropping user"; "user" => ?user);
            drop_user(user.uaid, app_state.db.as_ref(), &app_state.metrics).await?;
            return Err(ApiErrorKind::NoSubscription.into());
        }
    };

    if router_type == RouterType::WebPush {
        validate_webpush_user(user, channel_id, app_state.db.as_ref(), &app_state.metrics).await?;
    }

    Ok(router_type)
}

/// Make sure the user is not inactive and the subscription channel exists
async fn validate_webpush_user(
    user: &User,
    channel_id: &Uuid,
    db: &dyn DbClient,
    metrics: &StatsdClient,
) -> ApiResult<()> {
    if let Some(rotating_message_table) = db.rotating_message_table() {
        // DynamoDB: Make sure the user is active (has a valid message table)
        let Some(ref current_month) = user.current_month else {
            debug!("Missing `current_month` value, dropping user"; "user" => ?user);
            drop_user(user.uaid, db, metrics).await?;
            return Err(ApiErrorKind::NoSubscription.into());
        };

        if current_month != rotating_message_table {
            debug!("User is inactive, dropping user";
                   "db.rotating_message_table" => rotating_message_table,
                   "user.current_month" => current_month,
                   "user" => ?user);
            drop_user(user.uaid, db, metrics).await?;
            return Err(ApiErrorKind::NoSubscription.into());
        }
    }

    // Make sure the subscription channel exists
    let channel_ids = db.get_channels(&user.uaid).await?;

    if !channel_ids.contains(channel_id) {
        return Err(ApiErrorKind::NoSubscription.into());
    }

    Ok(())
}

/// Drop a user and increment associated metric
pub async fn drop_user(uaid: Uuid, db: &dyn DbClient, metrics: &StatsdClient) -> ApiResult<()> {
    metrics
        .incr_with_tags("updates.drop_user")
        .with_tag("errno", "102")
        .send();

    db.remove_user(&uaid).await?;

    Ok(())
}
