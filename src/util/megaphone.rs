use errors::Result;
use std::collections::HashMap;
use std::time::Duration;

use reqwest;

// A Broadcast entry Key in a BroadcastRegistry
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
struct BroadcastKey(u32);

// A list of broadcasts that a client is interested in and the last change seen
#[derive(Debug, Default)]
pub struct ClientBroadcasts {
    broadcast_list: Vec<BroadcastKey>,
    change_count: u32,
}

#[derive(Debug)]
struct BroadcastRegistry {
    lookup: HashMap<String, u32>,
    table: Vec<String>,
}

// Return result of the first delta call for a client given a full list of broadcast id's and versions
#[derive(Debug)]
pub struct BroadcastClientInit(pub ClientBroadcasts, pub Vec<Broadcast>);

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
            return BroadcastKey(*v);
        }
        let i = self.table.len();
        self.table.push(broadcast_id.clone());
        self.lookup.insert(broadcast_id, i as u32);
        BroadcastKey(i as u32)
    }

    fn lookup_id(&self, key: BroadcastKey) -> Option<String> {
        self.table.get(key.0 as usize).cloned()
    }

    fn lookup_key(&self, broadcast_id: &str) -> Option<BroadcastKey> {
        self.lookup.get(broadcast_id).cloned().map(BroadcastKey)
    }
}

// An individual broadcast and the current change count
#[derive(Debug)]
struct BroadcastRevision {
    change_count: u32,
    broadcast: BroadcastKey,
}

// A provided Broadcast/Version used for `ChangeList` initialization, client comparisons, and
// outgoing deltas
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Broadcast {
    broadcast_id: String,
    version: String,
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

impl From<Broadcast> for (String, String) {
    fn from(svc: Broadcast) -> (String, String) {
        (svc.broadcast_id, svc.version)
    }
}

impl Broadcast {
    pub fn from_hashmap(val: HashMap<String, String>) -> Vec<Broadcast> {
        val.into_iter().map(|v| v.into()).collect()
    }

    pub fn into_hashmap(broadcast_vec: Vec<Broadcast>) -> HashMap<String, String> {
        broadcast_vec.into_iter().map(|v| v.into()).collect()
    }
}

// BroadcastChangeTracker tracks the broadcasts, their change_count, and the broadcast lookup registry
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
        let mut svc_change_tracker = BroadcastChangeTracker {
            broadcast_list: Vec::new(),
            broadcast_registry: BroadcastRegistry::new(),
            broadcast_versions: HashMap::new(),
            change_count: 0,
        };
        for srv in broadcasts {
            let key = svc_change_tracker
                .broadcast_registry
                .add_broadcast(srv.broadcast_id);
            svc_change_tracker.broadcast_versions.insert(key, srv.version);
        }
        svc_change_tracker
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
            .header(reqwest::header::Authorization(token.to_string()))
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
            return change_count;
        }
        self.change_count += 1;
        let key = self.broadcast_registry.add_broadcast(broadcast.broadcast_id);
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
        let key = self.broadcast_registry
            .lookup_key(&broadcast.broadcast_id)
            .ok_or("Broadcast not found")?;

        if let Some(ver) = self.broadcast_versions.get_mut(&key) {
            if *ver == broadcast.version {
                return Ok(self.change_count);
            }
            *ver = broadcast.version;
        } else {
            return Err("Broadcast not found".into());
        }

        // Check to see if this broadcast has been updated since initialization
        let svc_index = self.broadcast_list
            .iter()
            .enumerate()
            .filter_map(|(i, svc)| if svc.broadcast == key { Some(i) } else { None })
            .nth(0);
        self.change_count += 1;
        if let Some(svc_index) = svc_index {
            let mut svc = self.broadcast_list.remove(svc_index);
            svc.change_count = self.change_count;
            self.broadcast_list.push(svc);
        } else {
            self.broadcast_list.push(BroadcastRevision {
                change_count: self.change_count,
                broadcast: key,
            })
        }
        Ok(self.change_count)
    }

    /// Returns the new broadcast versions since the provided `client_set`.
    pub fn change_count_delta(&self, client_set: &mut ClientBroadcasts) -> Option<Vec<Broadcast>> {
        if self.change_count <= client_set.change_count {
            return None;
        }
        let mut svc_delta = Vec::new();
        for svc in self.broadcast_list.iter().rev() {
            if svc.change_count <= client_set.change_count {
                break;
            }
            if !client_set.broadcast_list.contains(&svc.broadcast) {
                continue;
            }
            if let Some(ver) = self.broadcast_versions.get(&svc.broadcast) {
                if let Some(svc_id) = self.broadcast_registry.lookup_id(svc.broadcast) {
                    svc_delta.push(Broadcast {
                        broadcast_id: svc_id,
                        version: (*ver).clone(),
                    });
                }
            }
        }
        client_set.change_count = self.change_count;
        if svc_delta.is_empty() {
            None
        } else {
            Some(svc_delta)
        }
    }

    /// Returns a delta for `broadcasts` that are out of date with the latest version and a new
    /// `ClientSet``.
    pub fn broadcast_delta(&self, broadcasts: &[Broadcast]) -> BroadcastClientInit {
        let mut svc_list = Vec::new();
        let mut svc_delta = Vec::new();
        for svc in broadcasts.iter() {
            if let Some(svc_key) = self.broadcast_registry.lookup_key(&svc.broadcast_id) {
                if let Some(ver) = self.broadcast_versions.get(&svc_key) {
                    if *ver != svc.version {
                        svc_delta.push(Broadcast {
                            broadcast_id: svc.broadcast_id.clone(),
                            version: (*ver).clone(),
                        });
                    }
                }
                svc_list.push(svc_key);
            }
        }
        BroadcastClientInit(
            ClientBroadcasts {
                broadcast_list: svc_list,
                change_count: self.change_count,
            },
            svc_delta,
        )
    }

    /// Update a `ClientBroadcasts` to account for a new broadcast.
    ///
    /// Returns broadcasts that have changed.
    pub fn client_broadcast_add_broadcast(
        &self,
        client_broadcast: &mut ClientBroadcasts,
        broadcasts: &[Broadcast],
    ) -> Option<Vec<Broadcast>> {
        let mut svc_delta = self.change_count_delta(client_broadcast)
            .unwrap_or_default();
        for svc in broadcasts.iter() {
            if let Some(svc_key) = self.broadcast_registry.lookup_key(&svc.broadcast_id) {
                if let Some(ver) = self.broadcast_versions.get(&svc_key) {
                    if *ver != svc.version {
                        svc_delta.push(Broadcast {
                            broadcast_id: svc.broadcast_id.clone(),
                            version: (*ver).clone(),
                        });
                    }
                }
                client_broadcast.broadcast_list.push(svc_key)
            }
        }
        if svc_delta.is_empty() {
            None
        } else {
            Some(svc_delta)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_broadcast_base() -> Vec<Broadcast> {
        vec![
            Broadcast {
                broadcast_id: String::from("svca"),
                version: String::from("rev1"),
            },
            Broadcast {
                broadcast_id: String::from("svcb"),
                version: String::from("revalha"),
            },
        ]
    }

    #[test]
    fn test_broadcast_change_tracker() {
        let broadcasts = make_broadcast_base();
        let client_broadcasts = broadcasts.clone();
        let mut svc_chg_tracker = BroadcastChangeTracker::new(broadcasts);
        let BroadcastClientInit(mut client_svc, delta) =
            svc_chg_tracker.broadcast_delta(&client_broadcasts);
        assert_eq!(delta.len(), 0);
        assert_eq!(client_svc.change_count, 0);
        assert_eq!(client_svc.broadcast_list.len(), 2);

        svc_chg_tracker
            .update_broadcast(Broadcast {
                broadcast_id: String::from("svca"),
                version: String::from("rev2"),
            })
            .ok();
        let delta = svc_chg_tracker.change_count_delta(&mut client_svc);
        assert!(delta.is_some());
        let delta = delta.unwrap();
        assert_eq!(delta.len(), 1);
    }

    #[test]
    fn test_broadcast_change_handles_new_broadcasts() {
        let broadcasts = make_broadcast_base();
        let client_broadcasts = broadcasts.clone();
        let mut svc_chg_tracker = BroadcastChangeTracker::new(broadcasts);
        let BroadcastClientInit(mut client_svc, _) = svc_chg_tracker.broadcast_delta(&client_broadcasts);

        svc_chg_tracker.add_broadcast(Broadcast {
            broadcast_id: String::from("svcc"),
            version: String::from("revmega"),
        });
        let delta = svc_chg_tracker.change_count_delta(&mut client_svc);
        assert!(delta.is_none());

        let delta = svc_chg_tracker
            .client_broadcast_add_broadcast(
                &mut client_svc,
                &vec![
                    Broadcast {
                        broadcast_id: String::from("svcc"),
                        version: String::from("revision_alpha"),
                    },
                ],
            )
            .unwrap();
        assert_eq!(delta.len(), 1);
        assert_eq!(delta[0].version, String::from("revmega"));
        assert_eq!(client_svc.change_count, 1);
        assert_eq!(svc_chg_tracker.broadcast_list.len(), 1);
    }
}
