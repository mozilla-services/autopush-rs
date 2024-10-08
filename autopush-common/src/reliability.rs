/// Push Reliability Recorder
///
/// This allows us to track messages from select, known parties (currently, just
/// mozilla generated and consumed) so that we can identify potential trouble spots
/// and where messages expire early. Message expiration can lead to message loss
use std::sync::Arc;

use crate::db::client::DbClient;
use crate::errors::{ApcError, ApcErrorKind, Result};

pub const COUNTS: &str = "state_counts";
pub const ITEMS: &str = "items";

/// The various states that a message may transit on the way from reception to delivery.
#[derive(Debug, Clone, Copy)]
pub enum PushReliabilityState {
    RECEIVED,
    STORED,
    RETRIEVED,
    TRANSMITTED,
    ACCEPTED,
    DELIVERED,
}

// TODO: Differentiate between "transmitted via webpush" and "transmitted via bridge"?
impl std::fmt::Display for PushReliabilityState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::RECEIVED => "received",
            Self::STORED => "stored",
            Self::RETRIEVED => "retrieved",
            Self::TRANSMITTED => "transmitted",
            Self::ACCEPTED => "accepted",
            Self::DELIVERED => "delivered",
        })
    }
}

impl std::str::FromStr for PushReliabilityState {
    type Err = ApcError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "recieved" => Self::RECEIVED,
            "stored" => Self::STORED,
            "retrieved" => Self::RETRIEVED,
            "transmitted" => Self::TRANSMITTED,
            "accepted" => Self::ACCEPTED,
            "delivered" => Self::DELIVERED,
            _ => {
                return Err(ApcErrorKind::GeneralError("Unknown tracker state".to_owned()).into());
            }
        })
    }
}

#[derive(Default, Clone)]
pub struct PushReliability {
    client: Option<Arc<redis::Client>>,
    db: Option<Box<dyn DbClient>>,
}

impl PushReliability {
    // Do the magic to make a report instance, whatever that will be.
    pub fn new(record_dsn: &Option<String>, db: &Option<Box<dyn DbClient>>) -> Result<Self> {
        if record_dsn.is_none() {
            return Ok(Self::default());
        };

        let client = if let Some(dsn) = record_dsn {
            let rclient = redis::Client::open(dsn.clone()).map_err(|e| {
                ApcErrorKind::GeneralError(format!("Could not connect to redis server: {:?}", e))
            })?;
            Some(Arc::new(rclient))
        } else {
            None
        };

        Ok(Self {
            client,
            db: db.clone(),
        })
    }

    // Record the record state change to storage.
    pub async fn record(
        &self,
        reliability_id: &Option<String>,
        new: PushReliabilityState,
        old: &Option<PushReliabilityState>,
        expr: Option<u64>,
    ) -> Option<PushReliabilityState> {
        if reliability_id.is_none() {
            return None;
        }
        let id = reliability_id.clone().unwrap();
        if let Some(client) = &self.client {
            info!(
                "🔍 {} From {} to {}",
                id,
                old.map(|v| v.to_string())
                    .unwrap_or_else(|| "None".to_owned()),
                new
            );
            if let Ok(mut con) = client.get_connection() {
                let mut pipeline = redis::Pipeline::new();
                let pipeline = pipeline.hincr(COUNTS, new.to_string(), 1);
                let pipeline = if let Some(old) = old {
                    pipeline
                        .hincr(COUNTS, old.to_string(), -1)
                        .zrem(ITEMS, format!("{}#{}", &old, id))
                } else {
                    pipeline
                };
                // Errors are not fatal, and should not impact message flow, but
                // we should record them somewhere.
                let _ = pipeline
                    .zadd(ITEMS, format!("{}#{}", new, id), expr.unwrap_or_default())
                    .exec(&mut con)
                    .inspect_err(|e| {
                        warn!("🔍 Failed to write to storage: {:?}", e);
                    });
            }
        };
        if let Some(db) = &self.db {
            // Errors are not fatal, and should not impact message flow, but
            // we should record them somewhere.
            let _ = db.log_report(&id, new).await.inspect_err(|e| {
                warn!("🔍 Unable to record reliability state: {:?}", e);
            });
        }
        Some(new)
    }
}
