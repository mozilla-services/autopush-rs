use std::{collections::HashMap, sync::Arc, time::Duration};

use actix_web::rt;
use cadence::{CountedExt, StatsdClient};
use serde_derive::Deserialize;
use tokio::sync::RwLock;

use crate::broadcast::{Broadcast, BroadcastChangeTracker};

/// The payload provided by the Megaphone service
#[derive(Deserialize)]
pub struct MegaphoneResponse {
    pub broadcasts: HashMap<String, String>,
}

/// Initialize the `BroadcastChangeTracker`
///
/// Immediately populates it with the current Broadcasts polled from the
/// Megaphone service, then spawns a background task to periodically refresh
/// it
pub async fn init_and_spawn_megaphone_updater(
    broadcaster: &Arc<RwLock<BroadcastChangeTracker>>,
    http: &reqwest::Client,
    metrics: &Arc<StatsdClient>,
    url: &str,
    token: &str,
    poll_interval: Duration,
) -> reqwest::Result<()> {
    updater(broadcaster, http, url, token).await?;

    let broadcaster = Arc::clone(broadcaster);
    let http = http.clone();
    let metrics = Arc::clone(metrics);
    let url = url.to_owned();
    let token = token.to_owned();
    rt::spawn(async move {
        loop {
            rt::time::sleep(poll_interval).await;
            if let Err(e) = updater(&broadcaster, &http, &url, &token).await {
                report_updater_error(&metrics, e);
            } else {
                metrics.incr_with_tags("megaphone.updater.ok").send();
            }
        }
    });

    Ok(())
}

/// Emits a log, metric and Sentry event depending on the type of Error
fn report_updater_error(metrics: &Arc<StatsdClient>, err: reqwest::Error) {
    let reason = if err.is_timeout() {
        "timeout"
    } else if err.is_connect() {
        "connect"
    } else {
        "unknown"
    };
    metrics
        .incr_with_tags("megaphone.updater.error")
        .with_tag("reason", reason)
        .send();
    if reason == "unknown" {
        error!("📢megaphone::updater failed: {}", err);
        sentry::capture_event(sentry::event_from_error(&err));
    } else {
        trace!("📢megaphone::updater failed (reason: {}): {}", reason, err);
    }
}

/// Refresh the `BroadcastChangeTracker`'s Broadcasts from the Megaphone service
async fn updater(
    broadcaster: &Arc<RwLock<BroadcastChangeTracker>>,
    http: &reqwest::Client,
    url: &str,
    token: &str,
) -> reqwest::Result<()> {
    trace!("📢megaphone::updater");
    let MegaphoneResponse { broadcasts } = http
        .get(url)
        .header("Authorization", token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let broadcasts = Broadcast::from_hashmap(broadcasts);
    if !broadcasts.is_empty() {
        let change_count = broadcaster.write().await.add_broadcasts(broadcasts);
        trace!("📢 add_broadcast change_count: {:?}", change_count);
    }
    Ok(())
}
