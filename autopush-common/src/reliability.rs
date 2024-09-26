/// Push Reliability Recorder
use std::collections::HashMap;

use crate::errors::{ApcError, ApcErrorKind, Result};
use crate::util;
use redis::{AsyncCommands, Client};
use serde::Deserialize;

/// The various states that a message may transit on the way from reception to delivery.
#[derive(Debug, Clone, Deserialize)]
pub enum PushReliabilityState {
    RECEIVED,
    STORED,
    RETRIEVED,
    TRANSMITTED,
    ACCEPTED,
    DELIVERED,
}

const GC_LUA: &str = r#"
    local now = tonumber(ARGV[1])
    -- Collect all the items that have a score less than `now`
    local expired_items = redis.call('ZRANGE', 'items', '-inf', now, 'BYSCORE')

    for i, key in ipairs(expired_items) do
        local state,messageId = string.match(key, "(.*)%#(.*)")
        if state then
            redis.call('HINCRBY', 'state_counts', state, -1)
        end
        -- now clean things up.
        redis.call('ZREM', 'items', key)
        redis.call('DEL', key)
    end
    -- And return the items we removed.
    return expired_items
    "#;

const COUNTS: &str = "state_counts";
const ITEMS: &str = "items";

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
    scripts: HashMap<String, redis::Script>,
}

impl PushReliability {
    // Do the magic to make a report instance, whatever that will be.
    pub fn new(dsn: &str) -> Result<Self> {
        let client = dsn.is_empty().then(|| Client::open(dsn).unwrap());

        // register the scripts
        let mut scripts = HashMap::<String, redis::Script>::new();
        scripts.insert("gc".to_owned(), redis::Script::new(GC_LUA));

        Ok(Self { client, scripts })
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

        let mut expiry: HashMap<String, u64> = HashMap::new();
        expiry.insert(
            format!("{}#{}", new_state.to_string(), rid),
            util::sec_since_epoch() + expiry_s.unwrap_or_default(),
        );
        let mut pipe = redis::pipe();
        let pipe = pipe.atomic().hincr(COUNTS, new_state.to_string(), 1);
        let pipe = if let Some(old) = &prior_state {
            pipe.hincr(COUNTS, old.to_string(), -1)
                .zrem(ITEMS, format!("{}#{}", old.to_string(), rid))
        } else {
            pipe
        };
        pipe.zadd(ITEMS, expiry, 0)
            .exec_async(&mut con)
            .await
            .map_err(|e| e.into())
    }

    pub async fn garbage_collect(&self) -> Result<()> {
        if self.client.is_none() {
            return Ok(());
        }
        let mut con = self
            .client
            .as_ref()
            .unwrap()
            .get_multiplexed_async_connection()
            .await?;
        let purged: Vec<String> = con
            .zrange(ITEMS, -1, util::sec_since_epoch() as isize)
            .await?;
        let mut pipe = redis::pipe();
        let mut pipe = pipe.atomic();
        for key in purged {
            trace!("🔍 Purging: {}", key);
            let parts: Vec<&str> = key.splitn(2, '#').collect();
            let state = parts[0];
            pipe = pipe.hincr(COUNTS, state, -1).zrem(ITEMS, key);
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
