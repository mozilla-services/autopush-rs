use chrono::prelude::{DateTime, Utc};
use serde::{ser::SerializeStruct, Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, fmt::Display, time::SystemTime, vec::Vec};
use uuid::Uuid;

// TODO: We should probably denote that this is hand-rolled rust.
const PARSER_ID: &str = "glean_parser v15.0.1";

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
/// let glean_settings = GleanSettings {
///     application_id: GLEAN_APPLICATION_ID.to_owned(),
///     display_version: GLEAN_DISPLAY_VERSION.to_owned(),
///     channel: GLEAN_CHANNEL.to_owned(),
/// };
///
/// let mut metric_set = MetricSet::default()
///     .add_string("first_string", "First")?
///     .add_quantity("second_quantity", 1)?;
///
/// let glean = Glean::try_new(
///     &glean_settings,
///     "autotrack",        # the ping.yaml / metrics.yaml Category name
///     "some_event",       # the metrics.yaml Event descriptor name
///     metric_set,
///     None,
///     None,
///     )?.to_string();
///
/// print!("{}", glean);
///
/// ```
#[derive(serde::Serialize, Debug)]
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
    pub fn try_new(
        glean_settings: &GleanSettings,
        event: GleanEvent,
        ping_category: &str,

        metric_set: MetricSet,

        user_agent: Option<&str>, // These are optional (and should not be filled for autopush)
        ip_address: Option<&str>,
    ) -> Result<Self, serde_json::Error> {
        let now: DateTime<Utc> = SystemTime::now().into();

        let payload = PingPayload::try_new(
            &metric_set,
            event,
            &glean_settings.display_version,
            &glean_settings.channel,
            now,
        )?;
        Ok(Self {
            timestamp: now.to_rfc3339(),
            logger: "glean".to_owned(),
            gtype: "glean-server-event".to_owned(),
            fields: Ping::new(
                glean_settings,
                ping_category,
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

/// MetricSet is kind of the work-horse for defining the metrics being "recorded".
/// Ideally, this would validate the metrics against some yaml file and only allow
/// those that have been defined, in addition, it would also only allow metrics that
/// would be constructed for the various pings.
///
/// I am not that clever.
///
/// Glean offers two types of recordable data for Events, either "string" values
/// (which have a max length of 255 characters) or "quantity" (which are analogs
/// for counters and are Max i64)
/// You are to define each of these in the `metrics.yaml` file.
#[derive(Clone, Default)]
pub struct MetricSet {
    quantities: HashMap<String, Value>,
    strings: HashMap<String, Value>,
}

impl MetricSet {
    /// Set a single string value
    pub fn add_string(&mut self, key: &str, value: &str) -> Result<&mut Self, serde_json::Error> {
        let v = Value::from(value);
        self.strings.insert(key.to_owned(), v);
        Ok(self)
    }

    /// Set a single quantity value
    pub fn add_quantity(&mut self, key: &str, value: i64) -> Result<&mut Self, serde_json::Error> {
        self.quantities.insert(key.to_owned(), Value::from(value));
        Ok(self)
    }

    /// Set a batch of strings
    pub fn set_strings(
        &mut self,
        values: HashMap<&str, &str>,
    ) -> Result<&mut MetricSet, serde_json::Error> {
        for (key, value) in values.iter() {
            self.add_string(key, value)?;
        }
        Ok(self)
    }

    /// Set a batch of quantities
    pub fn set_quantities(
        &mut self,
        values: HashMap<&str, i64>,
    ) -> Result<&mut MetricSet, serde_json::Error> {
        for (key, value) in values.iter() {
            self.add_quantity(key, *value)?;
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

    // TODO: Add from_hashmap?
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
struct PingPayload {
    metrics: MetricSet,
    events: Vec<HashMap<String, Value>>,
    ping_info: HashMap<String, Value>,
    client_info: HashMap<String, Value>,
}

impl PingPayload {
    fn try_new(
        metrics: &MetricSet,
        event: GleanEvent,
        display_version: &str,
        channel: &str,
        timestamp: DateTime<Utc>,
    ) -> Result<Self, serde_json::error::Error> {
        let timestamp_str = timestamp.to_rfc3339();

        // Ping informatino is ad-hoc for servers, so these reflect that.
        let mut ping_info: HashMap<String, Value> = HashMap::new();
        ping_info.insert("seq".to_owned(), Value::from(0));
        ping_info.insert("start_time".to_owned(), Value::from(timestamp_str.as_str()));
        ping_info.insert("end_time".to_owned(), Value::from(timestamp_str.as_str()));

        // Most of these are "hard coded" for server apps.
        let mut client_info: HashMap<String, Value> = HashMap::new();
        client_info.insert("telemetry_sdk_build".to_owned(), Value::from(PARSER_ID));
        client_info.insert("first_run_date".to_owned(), Value::from("Unknown"));
        client_info.insert("os".to_owned(), Value::from("Unknown"));
        client_info.insert("os_version".to_owned(), Value::from("Unknown"));
        client_info.insert("architecture".to_owned(), Value::from("Unknown"));
        client_info.insert("app_build".to_owned(), Value::from("Unknown"));
        client_info.insert(
            "app_display_version".to_owned(),
            Value::from(display_version),
        );
        client_info.insert("app_channel".to_owned(), Value::from(channel));

        // Generate the ping payload that will be serialized in `Ping`
        Ok(Self {
            metrics: metrics.clone(),
            events: vec![event.as_event(&timestamp)?],
            ping_info,
            client_info,
        })
    }
}

/// Compose a "Ping" event.
#[derive(serde::Serialize, Debug)]
struct Ping {
    document_namespace: String,
    // Document type is the name of the Ping (defined in `pings.yml`)
    document_type: String,
    document_version: String,
    document_id: String,
    // For autopush, this is always "None"
    #[serde(skip_serializing_if = "Option::is_none")]
    user_agent: Option<String>,
    // For autopush, this is always "None"
    #[serde(skip_serializing_if = "Option::is_none")]
    ip_address: Option<String>,
    // Serialized `PingPayload`
    payload: String,
}

impl Ping {
    fn new(
        glean_settings: &GleanSettings,
        ping_category: &str,
        user_agent: Option<&str>,
        ip_address: Option<&str>,
        payload: &PingPayload,
    ) -> Self {
        Ping {
            document_namespace: glean_settings.application_id.to_owned(),
            document_type: ping_category.to_owned(),
            document_version: "1".to_owned(),
            document_id: Uuid::new_v4().to_string(),
            user_agent: user_agent.map(|v| v.to_owned()),
            ip_address: ip_address.map(|v| v.to_owned()),
            payload: serde_json::to_string(payload).unwrap(),
        }
    }
}

// These values are provided by Glean once the metrics & pings yml files
// are processed.
#[derive(Clone, Deserialize, Default, Debug, Serialize)]
pub struct GleanSettings {
    pub application_id: String,
    pub display_version: String,
    pub channel: String,
}

/// A Glean Event is how to categorize this submission. The server code
/// uses
#[derive(Clone, Deserialize, Default, Debug, Serialize)]
pub struct GleanEvent {
    pub category: &'static str, // the Category from metrics_server.yaml
    pub name: &'static str,     // the associated ping (from `send_in_pings` and pings.yaml)
}

impl GleanEvent {
    /// Return self as a serializable event.
    fn as_event(
        &self,
        timestamp: &DateTime<Utc>,
    ) -> Result<HashMap<String, Value>, serde_json::Error> {
        let mut event: HashMap<String, Value> = HashMap::new();

        event.insert(
            "timestamp".to_owned(),
            Value::from(timestamp.timestamp_millis()),
        );

        event.insert("category".to_owned(), Value::from(self.category));
        event.insert("name".to_owned(), Value::from(self.name));
        Ok(event)
    }
}

#[cfg(test)]
mod tests {

    use std::str::FromStr;
    use std::time::SystemTime;

    use super::{Glean, GleanSettings, MetricSet};
    use chrono::{DateTime, Utc};
    use serde_json::Value;

    fn parse_rfc3339(time: &str) -> SystemTime {
        let dt: DateTime<Utc> = time.parse().unwrap();
        SystemTime::from(dt)
    }

    #[test]
    fn test_glean() -> Result<(), serde_json::Error> {
        let now = SystemTime::now();

        let metric_set = MetricSet::default()
            .add_string("FirstString", "First")?
            .add_quantity("FirstQuant", 1)?
            .clone();

        let glean_settings = GleanSettings {
            application_id: "APPLICATION_ID".to_owned(),
            display_version: "DISPLAY_VERSION".to_owned(),
            channel: "CHANNEL".to_owned(),
        };

        let glean = Glean::try_new(
            &glean_settings,
            crate::glean::GleanEvent {
                category: "CategoryName",
                name: "EventName",
            },
            "AutoconnectPing",
            metric_set,
            None,
            None,
        )?;

        assert_eq!(&glean.fields.document_namespace, "APPLICATION_ID");
        assert_eq!(&glean.fields.document_type, "AutoconnectPing");
        assert_eq!(&glean.fields.user_agent, &None);
        assert_eq!(&glean.fields.ip_address, &None);
        assert!(!&glean.fields.document_id.is_empty());
        let de_payload = Value::from_str(&glean.fields.payload)?;
        assert_eq!(
            de_payload
                .get("client_info")
                .unwrap()
                .get("app_channel")
                .unwrap(),
            &Value::from("CHANNEL")
        );
        assert_eq!(
            de_payload
                .get("client_info")
                .unwrap()
                .get("app_display_version")
                .unwrap(),
            &Value::from("DISPLAY_VERSION")
        );
        assert_eq!(
            de_payload
                .get("events")
                .unwrap()
                .get(0)
                .unwrap()
                .get("category")
                .unwrap(),
            &Value::from("CategoryName")
        );
        assert_eq!(
            de_payload
                .get("events")
                .unwrap()
                .get(0)
                .unwrap()
                .get("name")
                .unwrap(),
            &Value::from("EventName")
        );
        assert_eq!(
            de_payload
                .get("metrics")
                .unwrap()
                .get("quantity")
                .unwrap()
                .get("FirstQuant")
                .unwrap(),
            &Value::from(1)
        );
        assert_eq!(
            de_payload
                .get("metrics")
                .unwrap()
                .get("string")
                .unwrap()
                .get("FirstString")
                .unwrap(),
            &Value::from("First")
        );

        let ts = parse_rfc3339(&glean.timestamp);
        assert!(ts > now);

        let pinfo = &de_payload.get("ping_info").unwrap();
        let sts = parse_rfc3339(pinfo.get("start_time").unwrap().as_str().unwrap());
        let ets = parse_rfc3339(pinfo.get("end_time").unwrap().as_str().unwrap());
        assert_eq!(sts, ets);
        assert!(sts > now);

        Ok(())
    }

    #[test]
    fn test_settings() -> Result<(), serde_json::Error> {
        let setting_string = "{\"application_id\": \"APPLICATION_ID\", \"display_version\": \"DISPLAY\", \"channel\": \"CHANNEL\"}";

        let settings: GleanSettings = serde_json::from_str(setting_string)?;

        assert_eq!(settings.application_id, "APPLICATION_ID");
        Ok(())
    }
}
