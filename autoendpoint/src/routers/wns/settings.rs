use serde;
use std::collections::HashMap;

/// Settings for `WnsRouter` These will be provided by secret storage.
#[derive(Clone, Default, Debug, serde::Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct WnsChannelSettings {
    pub tenant_id: String,
    pub app_id: String,
    pub secret: String,
}

/// This is the JSON formatted settings. This is a dict like:
/// ```{
///   "app_id_1": "{tenant_id: ..., app_id: ..., secret: ...}",
///   "app_id_2": "{tenant_id: ..., app_id: ..., secret: ...}"
/// }```
#[allow(unused)]
#[derive(Clone, Default, Debug, serde::Deserialize)]
pub struct WnsSettings {
    /// The App_ID to WNS credential mappings.
    pub channels: HashMap<String, WnsChannelSettings>,
    /// Max message size in bytes.
    pub max_data: usize,
    /// Max TTL for a WNS notification in seconds.
    pub max_ttl: u64,
}
