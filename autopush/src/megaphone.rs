use std::collections::HashMap;
use std::time::Duration;

use serde_derive::{Deserialize, Serialize};

use autopush_common::errors::{ApcErrorKind, Result};

use crate::server::protocol::BroadcastValue;

// A Broadcast entry Key in a BroadcastRegistry
type BroadcastKey = u32;

// Broadcasts a client is subscribed to and the last change seen
#[derive(Debug, Default)]
pub struct BroadcastSubs {
    broadcast_list: Vec<BroadcastKey>,
    change_count: u32,
}

#[derive(Debug)]
struct BroadcastRegistry {
    lookup: HashMap<String, BroadcastKey>,
    table: Vec<String>,
}

// Return result of the first delta call for a client given a full list of broadcast id's and
// versions
#[derive(Debug)]
pub struct BroadcastSubsInit(pub BroadcastSubs, pub Vec<Broadcast>);

impl BroadcastRegistry {
    fn new() -> BroadcastRegistry {
        BroadcastRegistry {
            lookup: HashMap::new(),
            table: Vec::new(),
        }
    }

    // Add's a new broadcast to the lookup table, returns the existing key if the broadcast already
    // exists
    fn add_broadcast(&mut self, broadcast_id: String) -> BroadcastKey {
        if let Some(v) = self.lookup.get(&broadcast_id) {
            return *v;
        }
        let i = self.table.len() as BroadcastKey;
        self.table.push(broadcast_id.clone());
        self.lookup.insert(broadcast_id, i);
        i
    }

    fn lookup_id(&self, key: BroadcastKey) -> Option<String> {
        self.table.get(key as usize).cloned()
    }

    fn lookup_key(&self, broadcast_id: &str) -> Option<BroadcastKey> {
        self.lookup.get(broadcast_id).copied()
    }
}

// An individual broadcast and the current change count
#[derive(Debug)]
struct BroadcastRevision {
    change_count: u32,
    broadcast: BroadcastKey,
}

// A provided Broadcast/Version used for `BroadcastSubsInit`, client comparisons, and outgoing
// deltas
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Broadcast {
    broadcast_id: String,
    version: String,
}

impl Broadcast {
    /// Errors out a broadcast for broadcasts that weren't found
    pub fn error(self) -> Broadcast {
        Broadcast {
            broadcast_id: self.broadcast_id,
            version: "Broadcast not found".to_string(),
        }
    }
}

// Handy From impls for common hashmap to/from conversions
impl From<(String, String)> for Broadcast {
    fn from(val: (String, String)) -> Broadcast {
        Broadcast {
            broadcast_id: val.0,
            version: val.1,
        }
    }
}

impl From<Broadcast> for (String, BroadcastValue) {
    fn from(bcast: Broadcast) -> (String, BroadcastValue) {
        (bcast.broadcast_id, BroadcastValue::Value(bcast.version))
    }
}

impl Broadcast {
    pub fn from_hashmap(val: HashMap<String, String>) -> Vec<Broadcast> {
        val.into_iter().map(|v| v.into()).collect()
    }

    pub fn vec_into_hashmap(broadcasts: Vec<Broadcast>) -> HashMap<String, BroadcastValue> {
        broadcasts.into_iter().map(|v| v.into()).collect()
    }
}

// BroadcastChangeTracker tracks the broadcasts, their change_count, and the broadcast lookup
// registry
#[derive(Debug)]
pub struct BroadcastChangeTracker {
    broadcast_list: Vec<BroadcastRevision>,
    broadcast_registry: BroadcastRegistry,
    broadcast_versions: HashMap<BroadcastKey, String>,
    change_count: u32,
}

#[derive(Deserialize)]
pub struct MegaphoneAPIResponse {
    pub broadcasts: HashMap<String, String>,
}

impl BroadcastChangeTracker {
    /// Creates a new `BroadcastChangeTracker` initialized with the provided `broadcasts`.
    pub fn new(broadcasts: Vec<Broadcast>) -> BroadcastChangeTracker {
        let mut tracker = BroadcastChangeTracker {
            broadcast_list: Vec::new(),
            broadcast_registry: BroadcastRegistry::new(),
            broadcast_versions: HashMap::new(),
            change_count: 0,
        };
        for srv in broadcasts {
            let key = tracker.broadcast_registry.add_broadcast(srv.broadcast_id);
            tracker.broadcast_versions.insert(key, srv.version);
        }
        tracker
    }

    /// Creates a new `BroadcastChangeTracker` initialized from a Megaphone API server version set
    /// as provided as the fetch URL.
    ///
    /// This method uses a synchronous HTTP call.
    pub fn with_api_broadcasts(url: &str, token: &str) -> reqwest::Result<BroadcastChangeTracker> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(1))
            .build()?;
        let MegaphoneAPIResponse { broadcasts } = client
            .get(url)
            .header("Authorization", token.to_string())
            .send()?
            .error_for_status()?
            .json()?;
        let broadcasts = Broadcast::from_hashmap(broadcasts);
        Ok(BroadcastChangeTracker::new(broadcasts))
    }

    /// Add a new broadcast to the BroadcastChangeTracker, triggering a change_count increase.
    /// Note: If the broadcast already exists, it will be updated instead.
    pub fn add_broadcast(&mut self, broadcast: Broadcast) -> u32 {
        if let Ok(change_count) = self.update_broadcast(broadcast.clone()) {
            trace!("📢 returning change count {}", &change_count);
            return change_count;
        }
        self.change_count += 1;
        let key = self
            .broadcast_registry
            .add_broadcast(broadcast.broadcast_id);
        self.broadcast_versions.insert(key, broadcast.version);
        self.broadcast_list.push(BroadcastRevision {
            change_count: self.change_count,
            broadcast: key,
        });
        self.change_count
    }

    /// Update a `broadcast` to a new revision, triggering a change_count increase.
    ///
    /// Returns an error if the `broadcast` was never initialized/added.
    pub fn update_broadcast(&mut self, broadcast: Broadcast) -> Result<u32> {
        let b_id = broadcast.broadcast_id.clone();
        let old_count = self.change_count;
        let key = self
            .broadcast_registry
            .lookup_key(&broadcast.broadcast_id)
            .ok_or_else(|| ApcErrorKind::BroadcastError("Broadcast not found".into()))?;

        if let Some(ver) = self.broadcast_versions.get_mut(&key) {
            if *ver == broadcast.version {
                return Ok(self.change_count);
            }
            *ver = broadcast.version;
        } else {
            trace!("📢 Not found: {}", &b_id);
            return Err(ApcErrorKind::BroadcastError("Broadcast not found".into()).into());
        }

        trace!("📢 New version of {}", &b_id);
        // Check to see if this broadcast has been updated since initialization
        let bcast_index = self
            .broadcast_list
            .iter()
            .enumerate()
            .filter_map(|(i, bcast)| {
                if bcast.broadcast == key {
                    Some(i)
                } else {
                    None
                }
            })
            .next();
        self.change_count += 1;
        if let Some(bcast_index) = bcast_index {
            trace!("📢  {} index: {}", &b_id, &bcast_index);
            let mut bcast = self.broadcast_list.remove(bcast_index);
            bcast.change_count = self.change_count;
            self.broadcast_list.push(bcast);
        } else {
            trace!("📢 adding broadcast list for {}", &b_id);
            self.broadcast_list.push(BroadcastRevision {
                change_count: self.change_count,
                broadcast: key,
            })
        }
        if old_count != self.change_count {
            trace!("📢 New Change available");
        }
        Ok(self.change_count)
    }

    /// Returns the new broadcast versions since the provided `client_set`.
    pub fn change_count_delta(&self, client_set: &mut BroadcastSubs) -> Option<Vec<Broadcast>> {
        if self.change_count <= client_set.change_count {
            return None;
        }
        let mut bcast_delta = Vec::new();
        for bcast in self.broadcast_list.iter().rev() {
            if bcast.change_count <= client_set.change_count {
                break;
            }
            if !client_set.broadcast_list.contains(&bcast.broadcast) {
                continue;
            }
            if let Some(ver) = self.broadcast_versions.get(&bcast.broadcast) {
                if let Some(bcast_id) = self.broadcast_registry.lookup_id(bcast.broadcast) {
                    bcast_delta.push(Broadcast {
                        broadcast_id: bcast_id,
                        version: (*ver).clone(),
                    });
                }
            }
        }
        client_set.change_count = self.change_count;
        if bcast_delta.is_empty() {
            None
        } else {
            Some(bcast_delta)
        }
    }

    /// Returns a delta for `broadcasts` that are out of date with the latest version and a
    /// the collection of broadcast subscriptions.
    pub fn broadcast_delta(&self, broadcasts: &[Broadcast]) -> BroadcastSubsInit {
        let mut bcast_list = Vec::new();
        let mut bcast_delta = Vec::new();
        for bcast in broadcasts.iter() {
            if let Some(bcast_key) = self.broadcast_registry.lookup_key(&bcast.broadcast_id) {
                if let Some(ver) = self.broadcast_versions.get(&bcast_key) {
                    if *ver != bcast.version {
                        bcast_delta.push(Broadcast {
                            broadcast_id: bcast.broadcast_id.clone(),
                            version: (*ver).clone(),
                        });
                    }
                }
                bcast_list.push(bcast_key);
            }
        }
        BroadcastSubsInit(
            BroadcastSubs {
                broadcast_list: bcast_list,
                change_count: self.change_count,
            },
            bcast_delta,
        )
    }

    /// Update a `BroadcastSubs` to account for new broadcasts.
    ///
    /// Returns broadcasts that have changed.
    pub fn subscribe_to_broadcasts(
        &self,
        broadcast_subs: &mut BroadcastSubs,
        broadcasts: &[Broadcast],
    ) -> Option<Vec<Broadcast>> {
        let mut bcast_delta = self.change_count_delta(broadcast_subs).unwrap_or_default();
        for bcast in broadcasts.iter() {
            if let Some(bcast_key) = self.broadcast_registry.lookup_key(&bcast.broadcast_id) {
                if let Some(ver) = self.broadcast_versions.get(&bcast_key) {
                    if *ver != bcast.version {
                        bcast_delta.push(Broadcast {
                            broadcast_id: bcast.broadcast_id.clone(),
                            version: (*ver).clone(),
                        });
                    }
                }
                broadcast_subs.broadcast_list.push(bcast_key)
            }
        }
        if bcast_delta.is_empty() {
            None
        } else {
            Some(bcast_delta)
        }
    }

    /// Check a broadcast list and return unknown broadcast id's with their appropriate error
    pub fn missing_broadcasts(&self, broadcasts: &[Broadcast]) -> Vec<Broadcast> {
        broadcasts
            .iter()
            .filter_map(|b| {
                if self
                    .broadcast_registry
                    .lookup_key(&b.broadcast_id)
                    .is_none()
                {
                    Some(b.clone().error())
                } else {
                    None
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_broadcast_base() -> Vec<Broadcast> {
        vec![
            Broadcast {
                broadcast_id: String::from("bcasta"),
                version: String::from("rev1"),
            },
            Broadcast {
                broadcast_id: String::from("bcastb"),
                version: String::from("revalha"),
            },
        ]
    }

    #[test]
    fn test_broadcast_change_tracker() {
        let broadcasts = make_broadcast_base();
        let desired_broadcasts = broadcasts.clone();
        let mut tracker = BroadcastChangeTracker::new(broadcasts);
        let BroadcastSubsInit(mut broadcast_subs, delta) =
            tracker.broadcast_delta(&desired_broadcasts);
        assert_eq!(delta.len(), 0);
        assert_eq!(broadcast_subs.change_count, 0);
        assert_eq!(broadcast_subs.broadcast_list.len(), 2);

        tracker
            .update_broadcast(Broadcast {
                broadcast_id: String::from("bcasta"),
                version: String::from("rev2"),
            })
            .ok();
        let delta = tracker.change_count_delta(&mut broadcast_subs);
        assert!(delta.is_some());
        let delta = delta.unwrap();
        assert_eq!(delta.len(), 1);
    }

    #[test]
    fn test_broadcast_change_handles_new_broadcasts() {
        let broadcasts = make_broadcast_base();
        let desired_broadcasts = broadcasts.clone();
        let mut tracker = BroadcastChangeTracker::new(broadcasts);
        let BroadcastSubsInit(mut broadcast_subs, _) = tracker.broadcast_delta(&desired_broadcasts);

        tracker.add_broadcast(Broadcast {
            broadcast_id: String::from("bcastc"),
            version: String::from("revmega"),
        });
        let delta = tracker.change_count_delta(&mut broadcast_subs);
        assert!(delta.is_none());

        let delta = tracker
            .subscribe_to_broadcasts(
                &mut broadcast_subs,
                &[Broadcast {
                    broadcast_id: String::from("bcastc"),
                    version: String::from("revision_alpha"),
                }],
            )
            .unwrap();
        assert_eq!(delta.len(), 1);
        assert_eq!(delta[0].version, String::from("revmega"));
        assert_eq!(broadcast_subs.change_count, 1);
        assert_eq!(tracker.broadcast_list.len(), 1);
    }
}
