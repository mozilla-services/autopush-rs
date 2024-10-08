/// Push Reliability Recorder
///
/// This allows us to track messages from select, known parties (currently, just
/// mozilla generated and consumed) so that we can identify potential trouble spots
/// and where messages expire early. Message expiration can lead to message loss
use std::collections::HashMap;
use std::sync::Arc;

use redis::Commands;

use crate::db::client::DbClient;
use crate::errors::{ApcError, ApcErrorKind, Result};

pub const COUNTS: &str = "state_counts";
pub const EXPIRY: &str = "expiry";

/// The various states that a message may transit on the way from reception to delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub enum PushReliabilityState {
    #[serde(rename = "received")]
    Received, // Subscription was received by the Push Server
    #[serde(rename = "stored")]
    Stored, // Subscription was stored because it could not be delivered immediately
    #[serde(rename = "retreived")]
    Retreived, // Subscription was taken from storage for delivery
    #[serde(rename = "transmitted_webpush")]
    IntTransmitted, // Subscription was handed off between autoendpoint and autoconnect
    #[serde(rename = "accepted_webpush")]
    IntAccepted, // Subscription was accepted by autoconnect from autopendpoint
    #[serde(rename = "transmitted")]
    Transmitted, // Subscription was handed off for delivery to the UA
    #[serde(rename = "accepted")]
    Accepted, // Subscription was accepted for delivery by the UA
    #[serde(rename = "delivered")]
    Delivered, // Subscription was provided to the WebApp recipient by the UA
    #[serde(rename = "expired")]
    Expired, // Subscription expired naturally (e.g. TTL=0)
}

// TODO: Differentiate between "transmitted via webpush" and "transmitted via bridge"?
impl std::fmt::Display for PushReliabilityState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Received => "received",
            Self::Stored => "stored",
            Self::Retreived => "retrieved",
            Self::Transmitted => "transmitted",
            Self::IntTransmitted => "transmitted_webpush",
            Self::IntAccepted => "accepted_webpush",
            Self::Accepted => "accepted",
            Self::Delivered => "delivered",
            Self::Expired => "expired",
        })
    }
}

impl std::str::FromStr for PushReliabilityState {
    type Err = ApcError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "received" => Self::Received,
            "stored" => Self::Stored,
            "retrieved" => Self::Retreived,
            "transmitted" => Self::Transmitted,
            "accepted" => Self::Accepted,
            "transmitted_webpush" => Self::IntTransmitted,
            "accepted_webpush" => Self::IntAccepted,
            "delivered" => Self::Delivered,
            "expired" => Self::Expired,
            _ => {
                return Err(
                    ApcErrorKind::GeneralError(format!("Unknown tracker state \"{}\"", s)).into(),
                );
            }
        })
    }
}

impl serde::Serialize for PushReliabilityState {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

#[derive(Default, Clone)]
pub struct PushReliability {
    client: Option<Arc<redis::Client>>,
    db: Option<Box<dyn DbClient>>,
}

impl PushReliability {
    // Do the magic to make a report instance, whatever that will be.
    pub fn new(reliability_dsn: &Option<String>, db: &Option<Box<dyn DbClient>>) -> Result<Self> {
        if reliability_dsn.is_none() {
            debug!("🔍 No reliability DSN declared.");
            return Ok(Self::default());
        };

        let client = if let Some(dsn) = reliability_dsn {
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
            debug!(
                "🔍 {} from {} to {}",
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
                        .zrem(EXPIRY, format!("{}#{}", &old, id))
                } else {
                    pipeline
                };
                // Errors are not fatal, and should not impact message flow, but
                // we should record them somewhere.
                let _ = pipeline
                    .zadd(EXPIRY, format!("{}#{}", new, id), expr.unwrap_or_default())
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

    // Return a snapshot of milestone states
    // This will probably not be called directly, but useful for debugging.
    pub async fn report(&self) -> Result<Option<HashMap<String, i32>>> {
        if let Some(client) = &self.client {
            if let Ok(mut conn) = client.get_connection() {
                return Ok(Some(conn.hgetall(COUNTS).map_err(|e| {
                    ApcErrorKind::GeneralError(format!("Could not read report {:?}", e))
                })?));
            }
        }
        Ok(None)
    }
}
