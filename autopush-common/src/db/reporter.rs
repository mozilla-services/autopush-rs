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

fn pool_periodic_reporter(db: &dyn DbClient, metrics: &StatsdClient, hostname: &str) {
    let Some(status) = db.pool_status() else {
        return;
    };
    metrics
        .gauge_with_tags(
            "database.pool.active",
            (status.size - status.available) as u64,
        )
        .with_tag("hostname", hostname)
        .send();
    metrics
        .gauge_with_tags("database.pool.idle", status.available as u64)
        .with_tag("hostname", hostname)
        .send();
    metrics
        .gauge_with_tags("database.pool.waiting", status.waiting as u64)
        .with_tag("hostname", hostname)
        .send();
}
