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
// Note: "Message" in this context refers to the Subscription Update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub enum ReliabilityState {
    #[serde(rename = "received")]
    Received, // Message was received by the Push Server
    #[serde(rename = "stored")]
    Stored, // Message was stored because it could not be delivered immediately
    #[serde(rename = "retrieved")]
    Retrieved, // Message was taken from storage for delivery
    #[serde(rename = "transmitted_webpush")]
    IntTransmitted, // Message was handed off between autoendpoint and autoconnect
    #[serde(rename = "accepted_webpush")]
    IntAccepted, // Message was accepted by autoconnect from autopendpoint
    #[serde(rename = "transmitted")]
    Transmitted, // Message was handed off for delivery to the UA
    #[serde(rename = "accepted")]
    Accepted, // Message was accepted for delivery by the UA
    #[serde(rename = "delivered")]
    Delivered, // Message was provided to the WebApp recipient by the UA
    #[serde(rename = "expired")]
    Expired, // Message expired naturally (e.g. TTL=0)
    #[serde(rename = "errored")]
    Errored, // Message errored out for some reason during delivery.
}

// TODO: Differentiate between "transmitted via webpush" and "transmitted via bridge"?
impl std::fmt::Display for ReliabilityState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Received => "received",
            Self::Stored => "stored",
            Self::Retrieved => "retrieved",
            Self::Transmitted => "transmitted",
            Self::IntTransmitted => "transmitted_webpush",
            Self::IntAccepted => "accepted_webpush",
            Self::Accepted => "accepted",
            Self::Delivered => "delivered",
            Self::Expired => "expired",
            Self::Errored => "errored",
        })
    }
}

impl std::str::FromStr for ReliabilityState {
    type Err = ApcError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "received" => Self::Received,
            "stored" => Self::Stored,
            "retrieved" => Self::Retrieved,
            "transmitted" => Self::Transmitted,
            "accepted" => Self::Accepted,
            "transmitted_webpush" => Self::IntTransmitted,
            "accepted_webpush" => Self::IntAccepted,
            "delivered" => Self::Delivered,
            "expired" => Self::Expired,
            "errored" => Self::Errored,
            _ => {
                return Err(
                    ApcErrorKind::GeneralError(format!("Unknown tracker state \"{}\"", s)).into(),
                );
            }
        })
    }
}

impl serde::Serialize for ReliabilityState {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

#[derive(Clone)]
pub struct PushReliability {
    client: Option<Arc<redis::Client>>,
    db: Box<dyn DbClient>,
}

impl PushReliability {
    // Do the magic to make a report instance, whatever that will be.
    pub fn new(reliability_dsn: &Option<String>, db: Box<dyn DbClient>) -> Result<Self> {
        if reliability_dsn.is_none() {
            debug!("🔍 No reliability DSN declared.");
            return Ok(Self {
                client: None,
                db: db.clone(),
            });
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
        new: ReliabilityState,
        old: &Option<ReliabilityState>,
        expr: Option<u64>,
    ) -> Option<ReliabilityState> {
        let Some(id) = reliability_id else {
            return None;
        };
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
        // Errors are not fatal, and should not impact message flow, but
        // we should record them somewhere.
        let _ = self.db.log_report(id, new).await.inspect_err(|e| {
            warn!("🔍 Unable to record reliability state: {:?}", e);
        });
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
