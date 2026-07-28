use std::{sync::Arc, time::Duration};

use actix_web::rt;
use cadence::{Gauged, StatsdClient};
use gethostname::gethostname;

use super::client::DbClient;

/// Emit db pool (deadpool) metrics periodically
pub fn spawn_pool_periodic_reporter(
    interval: Duration,
    db: Box<dyn DbClient>,
    metrics: Arc<StatsdClient>,
) {
    let hostname = gethostname().to_string_lossy().to_string();
    rt::spawn(async move {
        loop {
            pool_periodic_reporter(&*db, &metrics, &hostname);
            rt::time::sleep(interval).await;
        }
    });
}

fn pool_periodic_reporter(db: &dyn DbClient, metrics: &StatsdClient, _hostname: &str) {
    // The deadpool gauges count logical RPC slots, not connections. Naming them
    // "pool" invited reading them as a connection count, which they have not
    // been since channels were split out of pool entries.
    if let Some(status) = db.pool_status() {
        metrics
            .gauge_with_tags(
                "database.ops.inflight",
                (status.size - status.available) as u64,
            )
            //.with_tag("hostname", hostname)  // Do not include hostname due to cardinality
            .send();
        metrics
            .gauge_with_tags("database.ops.available", status.available as u64)
            .send();
        metrics
            .gauge_with_tags("database.ops.queued", status.waiting as u64)
            .send();
    }

    // One channel owns at most one HTTP/2 connection, so this is the ceiling on
    // sockets to Bigtable. Channels connect lazily, so a slot that has not yet
    // served an RPC has no socket and the true count can lag this at startup.
    if let Some(count) = db.configured_channel_count() {
        metrics
            .gauge_with_tags("database.channels", count as u64)
            .send();
    }
}
