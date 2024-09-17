use chrono::prelude::{DateTime, Utc};
use serde::ser::{Serialize, SerializeStruct};
use serde_json::Value;
use std::{collections::HashMap, fmt::Display, str::FromStr, time::SystemTime, vec::Vec};
use uuid::Uuid;

/// Construct a Glean compatible output string containing a recordable event.
///
/// Glean requires that all metrics be fully declared using the `metrics.yaml` and `pings.yaml` declaration file.
/// For this explanation, we will assume that `pings.yaml` declares a single pingtype of `autotrack` and
/// metrics.yaml declares a category of `autotrack`, with the abbreviate metric list of:
///  * `first_string` - type: "string"; send_in_pings: "autotrack"
///  * `second_string` - type: "string"; send_in_pings: "autotrack"
///  * `first_quantity` - type: "quantity"; send_in_pings: "autotrack"
///  * `second_quantity` - type: "quantity"; send_in_pings: "autotrack"
///
/// To record a Glean event
/// ```
/// # These values are provided by Glean once the application is approved.
/// const GLEAN_APPLICATION_ID:&str = "GleanApplicationId";
/// const GLEAN_DISPLAY_VERSION:&str = "GleanDisplayVersion";
/// const GLEAN_CHANNEL:&str = "GleanChannel";
///
/// let mut strings: HashMap<&str, &str> = HashMap::new();
/// let mut quantities: HashMap<&str, i64> = HashMap::new();
/// strings.insert("first_string", "First");
/// strings.insert("second_string", "Second");
/// quantities.insert("first_quantity", 1);
/// quantities.insert("second_quantity", 2);
///
/// let glean = Glean::try_new(
///     GLEAN_APPLICATION_ID,
///     GLEAN_DISPLAY_VERSION,
///     GLEAN_CHANNEL,
///     get_user_agent_string(),    # these functions are placeholders
///     get_ip_address_string(),    # use the client IP and UA string.
///     "autotrack",        # the ping.yaml / metrics.yaml Category name
///     "some_event",       # the metrics.yaml Event descriptor name
///     quantities,
///     strings)?.to_string();
///
/// print!("{}", glean);
///
/// ```
#[derive(serde::Serialize)]
pub struct Glean {
    #[serde(rename = "Timestamp")]
    timestamp: String,
    #[serde(rename = "Logger")]
    logger: String,
    #[serde(rename = "Type")]
    gtype: String,
    #[serde(rename = "Fields")]
    fields: Ping,
}

impl Glean {
    // Not a fan, but not sure how to best deal with this.
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        application_id: &str, //TODO: these should probably come from some `settings` bank
        display_version: &str,
        channel: &str,

        user_agent: &str, // TODO: These should come from the request bank
        ip_address: &str,

        category_name: &str,
        event_name: &str,
        quantities: Option<HashMap<&str, i64>>,
        strings: Option<HashMap<&str, &str>>,
    ) -> Result<Self, serde_json::Error> {
        let now: DateTime<Utc> = SystemTime::now().into();
        let metric_set = MetricSet::try_new(quantities, strings)?;
        let event = make_event(category_name, event_name)?;
        let payload = PingPayload::try_new(metric_set, event, display_version, channel, now)?;
        Ok(Self {
            timestamp: now.to_rfc3339(),
            logger: "glean".to_owned(),
            gtype: "glean-server-event".to_owned(),
            fields: Ping::new(
                application_id,
                category_name,
                user_agent,
                ip_address,
                &payload,
            ),
        })
    }
}

impl Display for Glean {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&serde_json::to_string(self).unwrap())
    }
}

pub(crate) fn make_event(
    category: &str,
    name: &str,
) -> Result<HashMap<String, Value>, serde_json::Error> {
    let mut event: HashMap<String, Value> = HashMap::new();

    event.insert("category".to_owned(), Value::from_str(category)?);
    event.insert("name".to_owned(), Value::from_str(name)?);
    Ok(event)
}

/// Glean offers two types of recordable data for Events, either "string" values
/// (which have a max length of 255 characters) or "quantity" (which are analogs for counters)
/// You are to define each of these in the `metrics.yaml` file.
#[derive(Default)]
pub struct MetricSet {
    quantities: HashMap<String, Value>,
    strings: HashMap<String, Value>,
}

impl MetricSet {
    /// Set a single string value
    pub fn set_string(&mut self, key: &str, value: &str) -> Result<(), serde_json::Error> {
        self.strings.insert(key.to_owned(), Value::from_str(value)?);
        Ok(())
    }

    /// Set a single quantity value
    pub fn set_quantity(&mut self, key: &str, value: &i64) -> Result<(), serde_json::Error> {
        self.quantities.insert(key.to_owned(), Value::from(*value));
        Ok(())
    }

    /// Set a batch of strings
    pub fn set_strings(
        &mut self,
        values: HashMap<&str, &str>,
    ) -> Result<&mut MetricSet, serde_json::Error> {
        for (key, value) in values.iter() {
            self.set_string(key, value)?;
        }
        Ok(self)
    }

    /// Set a batch of quantities
    pub fn set_quantities(
        &mut self,
        values: HashMap<&str, i64>,
    ) -> Result<&mut MetricSet, serde_json::Error> {
        for (key, value) in values.iter() {
            self.set_quantity(key, value)?;
        }
        Ok(self)
    }

    /// Convenience function to set all quantity and string values desired.
    pub fn try_new(
        quantities: Option<HashMap<&str, i64>>,
        strings: Option<HashMap<&str, &str>>,
    ) -> Result<Self, serde_json::Error> {
        let mut metric_set = Self::default();
        if let Some(quantities) = quantities {
            metric_set.set_quantities(quantities)?;
        }
        if let Some(strings) = strings {
            metric_set.set_strings(strings)?;
        }
        Ok(metric_set)
    }
}

impl Serialize for MetricSet {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("metrics", 2)?;
        state.serialize_field("quantity", &self.quantities)?;
        state.serialize_field("string", &self.strings)?;

        state.end()
    }
}

/// The "ping payload" is the content of the "Ping" event. In `server.yaml`,
/// fields maked as `send_in_pings` should only include those fields in the associated
/// Ping category. We're a bit more lax about that here, but we should check to see
/// if that complicates things.
#[derive(serde::Serialize)]
pub(crate) struct PingPayload {
    metrics: MetricSet,
    events: Vec<HashMap<String, Value>>,
    ping_info: HashMap<String, Value>,
    client_info: HashMap<String, Value>,
}

impl PingPayload {
    fn try_new(
        metrics: MetricSet,
        event: HashMap<String, Value>,
        display_version: &str,
        channel: &str,
        timestamp: DateTime<Utc>,
    ) -> Result<Self, serde_json::error::Error> {
        let timestamp_str = timestamp.to_rfc3339();
        let event_ms = timestamp.timestamp_millis();
        let mut mevent = event.clone();
        mevent.insert("timestamp".to_owned(), Value::from(event_ms));
        let mut ping_info: HashMap<String, Value> = HashMap::new();
        ping_info.insert("seq".to_owned(), Value::from(0));
        ping_info.insert("start_time".to_owned(), Value::from_str(&timestamp_str)?);
        ping_info.insert("end_time".to_owned(), Value::from_str(&timestamp_str)?);

        let mut client_info: HashMap<String, Value> = HashMap::new();
        client_info.insert(
            "telemetry_sdk_build".to_owned(),
            Value::from_str("clean_parser v15.0.1")?,
        );
        client_info.insert("first_run_date".to_owned(), Value::from_str("Unknown")?);
        client_info.insert("os".to_owned(), Value::from_str("Unknown")?);
        client_info.insert("os_version".to_owned(), Value::from_str("Unknown")?);
        client_info.insert("architecture".to_owned(), Value::from_str("Unknown")?);
        client_info.insert("app_build".to_owned(), Value::from_str("Unknown")?);
        client_info.insert(
            "app_display_version".to_owned(),
            Value::from_str(display_version)?,
        );
        client_info.insert("app_channel".to_owned(), Value::from_str(channel)?);

        Ok(Self {
            metrics,
            events: vec![mevent],
            ping_info,
            client_info,
        })
    }
}

/// Compose a "Ping" event.
#[derive(serde::Serialize)]
pub struct Ping {
    document_namespace: String,
    document_type: String,
    document_version: String,
    document_id: String,
    user_agent: String,
    ip_address: String,
    payload: String,
}

impl Ping {
    fn new(
        application_id: &str,
        ping_category: &str,
        user_agent: &str,
        ip_address: &str,
        payload: &PingPayload,
    ) -> Self {
        Ping {
            document_namespace: application_id.to_owned(),
            document_type: ping_category.to_owned(),
            document_version: "1".to_owned(),
            document_id: Uuid::new_v4().to_string(),
            user_agent: user_agent.to_owned(),
            ip_address: ip_address.to_owned(),
            payload: serde_json::to_string(payload).unwrap(),
        }
    }
}
