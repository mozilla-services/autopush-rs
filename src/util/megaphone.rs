use errors::Result;
use std::collections::HashMap;
use std::time::Duration;

use reqwest;

// A Service entry Key in a ServiceRegistry
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
struct ServiceKey(u32);

// A list of services that a client is interested in and the last change seen
#[derive(Debug, Default)]
pub struct ClientServices {
    service_list: Vec<ServiceKey>,
    change_count: u32,
}

#[derive(Debug)]
struct ServiceRegistry {
    lookup: HashMap<String, u32>,
    table: Vec<String>,
}

// Return result of the first delta call for a client given a full list of service id's and versions
#[derive(Debug)]
pub struct ServiceClientInit(pub ClientServices, pub Vec<Service>);

impl ServiceRegistry {
    fn new() -> ServiceRegistry {
        ServiceRegistry {
            lookup: HashMap::new(),
            table: Vec::new(),
        }
    }

    // Add's a new service to the lookup table, returns the existing key if the service already
    // exists
    fn add_service(&mut self, service_id: String) -> ServiceKey {
        if let Some(v) = self.lookup.get(&service_id) {
            return ServiceKey(*v);
        }
        let i = self.table.len();
        self.table.push(service_id.clone());
        self.lookup.insert(service_id, i as u32);
        ServiceKey(i as u32)
    }

    fn lookup_id(&self, key: ServiceKey) -> Option<String> {
        self.table.get(key.0 as usize).cloned()
    }

    fn lookup_key(&self, service_id: &str) -> Option<ServiceKey> {
        self.lookup.get(service_id).cloned().map(ServiceKey)
    }
}

// An individual service and the current change count
#[derive(Debug)]
struct ServiceRevision {
    change_count: u32,
    service: ServiceKey,
}

// A provided Service/Version used for `ChangeList` initialization, client comparisons, and
// outgoing deltas
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Service {
    service_id: String,
    version: String,
}

// Handy From impls for common hashmap to/from conversions
impl From<(String, String)> for Service {
    fn from(val: (String, String)) -> Service {
        Service {
            service_id: val.0,
            version: val.1,
        }
    }
}

impl From<Service> for (String, String) {
    fn from(svc: Service) -> (String, String) {
        (svc.service_id, svc.version)
    }
}

impl Service {
    pub fn from_hashmap(val: HashMap<String, String>) -> Vec<Service> {
        val.into_iter().map(|v| v.into()).collect()
    }

    pub fn into_hashmap(service_vec: Vec<Service>) -> HashMap<String, String> {
        service_vec.into_iter().map(|v| v.into()).collect()
    }
}

// ServiceChangeTracker tracks the services, their change_count, and the service lookup registry
#[derive(Debug)]
pub struct ServiceChangeTracker {
    service_list: Vec<ServiceRevision>,
    service_registry: ServiceRegistry,
    service_versions: HashMap<ServiceKey, String>,
    change_count: u32,
}

#[derive(Deserialize)]
pub struct MegaphoneAPIResponse {
    pub broadcasts: HashMap<String, String>,
}

impl ServiceChangeTracker {
    /// Creates a new `ServiceChangeTracker` initialized with the provided `services`.
    pub fn new(services: Vec<Service>) -> ServiceChangeTracker {
        let mut svc_change_tracker = ServiceChangeTracker {
            service_list: Vec::new(),
            service_registry: ServiceRegistry::new(),
            service_versions: HashMap::new(),
            change_count: 0,
        };
        for srv in services {
            let key = svc_change_tracker
                .service_registry
                .add_service(srv.service_id);
            svc_change_tracker.service_versions.insert(key, srv.version);
        }
        svc_change_tracker
    }

    /// Creates a new `ServiceChangeTracker` initialized from a Megaphone API server version set
    /// as provided as the fetch URL.
    ///
    /// This method uses a synchronous HTTP call.
    pub fn with_api_services(url: &str, token: &str) -> reqwest::Result<ServiceChangeTracker> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(1))
            .build()?;
        let MegaphoneAPIResponse { broadcasts } = client
            .get(url)
            .header(reqwest::header::Authorization(token.to_string()))
            .send()?
            .error_for_status()?
            .json()?;
        let services = Service::from_hashmap(broadcasts);
        Ok(ServiceChangeTracker::new(services))
    }

    /// Add a new service to the ServiceChangeTracker, triggering a change_count increase.
    /// Note: If the service already exists, it will be updated instead.
    pub fn add_service(&mut self, service: Service) -> u32 {
        if let Ok(change_count) = self.update_service(service.clone()) {
            return change_count;
        }
        self.change_count += 1;
        let key = self.service_registry.add_service(service.service_id);
        self.service_versions.insert(key, service.version);
        self.service_list.push(ServiceRevision {
            change_count: self.change_count,
            service: key,
        });
        self.change_count
    }

    /// Update a `service` to a new revision, triggering a change_count increase.
    ///
    /// Returns an error if the `service` was never initialized/added.
    pub fn update_service(&mut self, service: Service) -> Result<u32> {
        let key = self.service_registry
            .lookup_key(&service.service_id)
            .ok_or("Service not found")?;

        if let Some(ver) = self.service_versions.get_mut(&key) {
            if *ver == service.version {
                return Ok(self.change_count);
            }
            *ver = service.version;
        } else {
            return Err("Service not found".into());
        }

        // Check to see if this service has been updated since initialization
        let svc_index = self.service_list
            .iter()
            .enumerate()
            .filter_map(|(i, svc)| if svc.service == key { Some(i) } else { None })
            .nth(0);
        self.change_count += 1;
        if let Some(svc_index) = svc_index {
            let mut svc = self.service_list.remove(svc_index);
            svc.change_count = self.change_count;
            self.service_list.push(svc);
        } else {
            self.service_list.push(ServiceRevision {
                change_count: self.change_count,
                service: key,
            })
        }
        Ok(self.change_count)
    }

    /// Returns the new service versions since the provided `client_set`.
    pub fn change_count_delta(&self, client_set: &mut ClientServices) -> Option<Vec<Service>> {
        if self.change_count <= client_set.change_count {
            return None;
        }
        let mut svc_delta = Vec::new();
        for svc in self.service_list.iter().rev() {
            if svc.change_count <= client_set.change_count {
                break;
            }
            if !client_set.service_list.contains(&svc.service) {
                continue;
            }
            if let Some(ver) = self.service_versions.get(&svc.service) {
                if let Some(svc_id) = self.service_registry.lookup_id(svc.service) {
                    svc_delta.push(Service {
                        service_id: svc_id,
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

    /// Returns a delta for `services` that are out of date with the latest version and a new
    /// `ClientSet``.
    pub fn service_delta(&self, services: &[Service]) -> ServiceClientInit {
        let mut svc_list = Vec::new();
        let mut svc_delta = Vec::new();
        for svc in services.iter() {
            if let Some(svc_key) = self.service_registry.lookup_key(&svc.service_id) {
                if let Some(ver) = self.service_versions.get(&svc_key) {
                    if *ver != svc.version {
                        svc_delta.push(Service {
                            service_id: svc.service_id.clone(),
                            version: (*ver).clone(),
                        });
                    }
                }
                svc_list.push(svc_key);
            }
        }
        ServiceClientInit(
            ClientServices {
                service_list: svc_list,
                change_count: self.change_count,
            },
            svc_delta,
        )
    }

    /// Update a `ClientServices` to account for a new service.
    ///
    /// Returns services that have changed.
    pub fn client_service_add_service(
        &self,
        client_service: &mut ClientServices,
        services: &[Service],
    ) -> Option<Vec<Service>> {
        let mut svc_delta = self.change_count_delta(client_service)
            .unwrap_or_default();
        for svc in services.iter() {
            if let Some(svc_key) = self.service_registry.lookup_key(&svc.service_id) {
                if let Some(ver) = self.service_versions.get(&svc_key) {
                    if *ver != svc.version {
                        svc_delta.push(Service {
                            service_id: svc.service_id.clone(),
                            version: (*ver).clone(),
                        });
                    }
                }
                client_service.service_list.push(svc_key)
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

    fn make_service_base() -> Vec<Service> {
        vec![
            Service {
                service_id: String::from("svca"),
                version: String::from("rev1"),
            },
            Service {
                service_id: String::from("svcb"),
                version: String::from("revalha"),
            },
        ]
    }

    #[test]
    fn test_service_change_tracker() {
        let services = make_service_base();
        let client_services = services.clone();
        let mut svc_chg_tracker = ServiceChangeTracker::new(services);
        let ServiceClientInit(mut client_svc, delta) =
            svc_chg_tracker.service_delta(&client_services);
        assert_eq!(delta.len(), 0);
        assert_eq!(client_svc.change_count, 0);
        assert_eq!(client_svc.service_list.len(), 2);

        svc_chg_tracker
            .update_service(Service {
                service_id: String::from("svca"),
                version: String::from("rev2"),
            })
            .ok();
        let delta = svc_chg_tracker.change_count_delta(&mut client_svc);
        assert!(delta.is_some());
        let delta = delta.unwrap();
        assert_eq!(delta.len(), 1);
    }

    #[test]
    fn test_service_change_handles_new_services() {
        let services = make_service_base();
        let client_services = services.clone();
        let mut svc_chg_tracker = ServiceChangeTracker::new(services);
        let ServiceClientInit(mut client_svc, _) = svc_chg_tracker.service_delta(&client_services);

        svc_chg_tracker.add_service(Service {
            service_id: String::from("svcc"),
            version: String::from("revmega"),
        });
        let delta = svc_chg_tracker.change_count_delta(&mut client_svc);
        assert!(delta.is_none());

        let delta = svc_chg_tracker
            .client_service_add_service(
                &mut client_svc,
                &vec![
                    Service {
                        service_id: String::from("svcc"),
                        version: String::from("revision_alpha"),
                    },
                ],
            )
            .unwrap();
        assert_eq!(delta.len(), 1);
        assert_eq!(delta[0].version, String::from("revmega"));
        assert_eq!(client_svc.change_count, 1);
        assert_eq!(svc_chg_tracker.service_list.len(), 1);
    }
}
