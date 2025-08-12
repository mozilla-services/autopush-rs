//! User validations

use crate::error::{ApiErrorKind, ApiResult};
use crate::extractors::routers::RouterType;
use crate::server::AppState;
use actix_http::StatusCode;
use autopush_common::db::{client::DbClient, User};
use autopush_common::metric_name::MetricName;
use autopush_common::metrics::StatsdClientExt;
use cadence::StatsdClient;
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

    // Legacy GCM support was discontinued by Google in Sept 2023.
    // Since we do not have access to the account that originally created the GCM project
    // and credentials, we cannot move those users to modern FCM implementations, so we
    // must drop them.
    if router_type == RouterType::GCM {
        debug!("Encountered GCM record, dropping user"; "user" => ?user);
        // record the bridge error for accounting reasons.
        app_state
            .metrics
            .incr_with_tags(MetricName::NotificationBridgeError)
            .with_tag("platform", "gcm")
            .with_tag("reason", "gcm_kill")
            .with_tag("error", &StatusCode::GONE.to_string())
            .send();
        drop_user(user.uaid, app_state.db.as_ref(), &app_state.metrics).await?;
        return Err(ApiErrorKind::Router(crate::routers::RouterError::NotFound).into());
    }

    if router_type == RouterType::WebPush {
        validate_webpush_user(user, channel_id, app_state.db.as_ref()).await?;
    }

    Ok(router_type)
}

/// Make sure the user is not inactive and the subscription channel exists
async fn validate_webpush_user(user: &User, channel_id: &Uuid, db: &dyn DbClient) -> ApiResult<()> {
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
        .incr_with_tags(MetricName::UpdatesDropUser)
        .with_tag("errno", "102")
        .send();

    db.remove_user(&uaid).await?;

    Ok(())
}
