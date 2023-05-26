use std::{collections::HashMap, sync::Arc, time::Duration};

use actix_web::rt;
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
    url: &str,
    token: &str,
    poll_interval: Duration,
) -> reqwest::Result<()> {
    megaphone_updater(broadcaster, http, url, token).await?;

    let broadcaster = Arc::clone(broadcaster);
    let http = http.clone();
    let url = url.to_owned();
    let token = token.to_owned();
    rt::spawn(async move {
        loop {
            rt::time::sleep(poll_interval).await;
            if let Err(e) = megaphone_updater(&broadcaster, &http, &url, &token).await {
                error!("📢megaphone_updater failed: {}", e);
                sentry::capture_event(sentry::event_from_error(&e));
            }
        }
    });

    Ok(())
}

/// Refresh the `BroadcastChangeTracker`'s Broadcasts from the Megaphone service
async fn megaphone_updater(
    broadcaster: &Arc<RwLock<BroadcastChangeTracker>>,
    http: &reqwest::Client,
    url: &str,
    token: &str,
) -> reqwest::Result<()> {
    trace!("📢BroadcastChangeTracker::megaphone_updater");
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
