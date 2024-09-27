/// Push Reliability Recorder
use std::collections::HashMap;
use std::str::FromStr;

use crate::errors::{ApcError, ApcErrorKind, Result};
use crate::util;
use redis::{AsyncCommands, Client};
use serde::Deserialize;

/// The various states that a message may transit on the way from reception to delivery.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
pub enum PushReliabilityState {
    RECEIVED,
    STORED,
    RETRIEVED,
    TRANSMITTED,
    ACCEPTED,
    DELIVERED,
}

const COUNTS: &str = "state_counts";
const ITEMS: &str = "items";
const TOTALS: &str = "totals";
const TOTAL_SUCCESS: &str = "total_success";
const TOTAL_FAILURE: &str = "total_failure";

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

    fn from_str(s: &str) -> Result<Self> {
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

#[derive(Debug, Default, Clone)]
pub struct PushReliability {
    client: Option<Client>,
}

impl PushReliability {
    // Do the magic to make a report instance, whatever that will be.
    pub fn new(dsn: &str) -> Result<Self> {
        let client = dsn.is_empty().then(|| Client::open(dsn).unwrap());

        Ok(Self { client })
    }

    // Handle errors internally.
    pub async fn record(
        &self,
        // The unique identifier.
        reliability_id: &Option<String>,
        // The new state for the item.
        new_state: PushReliabilityState,
        prior_state: &Option<PushReliabilityState>,
        // The expected expiration timestamp.
        expiry_s: Option<u64>,
    ) -> Result<()> {
        if reliability_id.is_none() {
            return Ok(());
        }
        if self.client.is_none() {
            debug!(
                "🔍 Skipping report for {} from {} to {}",
                reliability_id.as_ref().unwrap_or(&"None".to_owned()),
                prior_state
                    .as_ref()
                    .map(|v| v.to_string())
                    .unwrap_or("New".to_owned()),
                new_state
            );
            return Ok(());
        }

        let rid = reliability_id.as_ref().unwrap();

        let mut con = self
            .client
            .as_ref()
            .unwrap()
            .get_multiplexed_async_connection()
            .await?;

        let mut pipe = redis::pipe();
        let pipe = pipe.atomic().hincr(COUNTS, new_state.to_string(), 1);

        // if we have a prior state, decrement that state's count, and remove the old state
        // milestone.
        let pipe = if let Some(old) = &prior_state {
            pipe.hincr(COUNTS, old.to_string(), -1)
                .zrem(ITEMS, format!("{}#{}", old, rid))
        } else {
            pipe
        };

        // Now, record the new milestone entry for this message.
        // The score should (hopefully) be meaningless here since there should
        // only ever be one entry with this hash value.
        let mut expiry: HashMap<String, u64> = HashMap::new();
        expiry.insert(
            format!("{}#{}", new_state, rid),
            util::sec_since_epoch() + expiry_s.unwrap_or_default(),
        );
        pipe.zadd(ITEMS, expiry, 0)
            .exec_async(&mut con)
            .await
            .map_err(|e| e.into())
    }

    pub async fn garbage_collect(&self) -> Result<()> {
        if self.client.is_none() {
            return Ok(());
        }

        // TODO: Lock redis for gc update?
        // Generate a lock, apply if not present, validate that the lock value matches what we set.
        // proceed.

        let mut con = self
            .client
            .as_ref()
            .unwrap()
            .get_multiplexed_async_connection()
            .await?;
        let mut success: u64 = 0;
        let mut fail: u64 = 0;

        // Collect up all the hash keys that have values (timestamps) that are less than "now"
        let purged: Vec<String> = con
            .zrange(ITEMS, -1, util::sec_since_epoch() as isize)
            .await?;
        let mut pipe = redis::pipe();
        let mut pipe = pipe.atomic();
        // iterate though the detected list of candidates and remove them, adjusting the counts.

        for key in purged {
            trace!("🔍 Purging: {}", key);
            let parts: Vec<&str> = key.splitn(2, '#').collect();
            let state = parts[0];
            pipe = pipe.hincr(COUNTS, state, -1).zrem(ITEMS, &key);
            if PushReliabilityState::from_str(state)? != PushReliabilityState::DELIVERED {
                success += 1;
            } else {
                fail += 1;
            };
        }
        // Record the overall success/failure counts A failure is anything that did not reach
        // "delivered" and was pruned.
        // We don't expire counts (that needs to be done externally.)
        if success > 0 {
            pipe = pipe.hincr(TOTALS, TOTAL_SUCCESS, success);
        }
        if fail > 0 {
            pipe = pipe.hincr(TOTALS, TOTAL_FAILURE, fail);
        }
        pipe.exec_async(&mut con).await.map_err(|e| e.into())
    }

    pub async fn counts(&self) -> Result<HashMap<String, u64>> {
        if self.client.is_none() {
            return Ok(HashMap::new());
        }
        let mut con = self
            .client
            .as_ref()
            .unwrap()
            .get_multiplexed_async_connection()
            .await?;

        Ok(con.hgetall(COUNTS).await?)
    }
}
