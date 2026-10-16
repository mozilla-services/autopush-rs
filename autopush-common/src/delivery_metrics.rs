//! Aggregate delivery latency and outcome metrics.
//!
//! These answer "is push slow?" and "is push reliable?" for all traffic without
//! tracking individual messages. Latency is recorded in Prometheus histograms
//! (served from `/metrics`) so quantiles can be aggregated across pods and time
//! windows. Terminal outcomes are statsd counters (`notification.outcome`).
use cadence::{Counted, StatsdClient};
use prometheus_client::{
    encoding::{EncodeLabelSet, EncodeLabelValue, LabelValueEncoder, text::encode},
    metrics::{family::Family, histogram::Histogram},
    registry::{Registry, Unit},
};

use crate::metric_name::MetricName;

/// Latency histogram bucket boundaries, in seconds.
///
/// Log-spaced from 10ms to the 30 day maximum notification TTL, so both direct
/// delivery (ms) and offline devices (hours/days) are resolvable. Both
/// autoendpoint and autoconnect use these so the series are comparable.
/// Changing these breaks historical comparison.
pub const DELIVERY_BUCKETS: [f64; 24] = [
    0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 300.0, 900.0, 1800.0, 3600.0,
    10800.0, 21600.0, 43200.0, 86400.0, 172800.0, 345600.0, 604800.0, 1209600.0, 2592000.0,
];

/// Content type for the `/metrics` response.
pub const CONTENT_TYPE: &str = "application/openmetrics-text; version=1.0.0; charset=utf-8";

/// The destination router of a notification.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum DeliveryPath {
    /// Delivered over a WebSocket by autoconnect.
    Ws,
    Fcm,
    Apns,
    /// The router could not be determined (e.g. the subscription could not be
    /// decoded). Only used for `notification.outcome`.
    Unknown,
}

impl DeliveryPath {
    pub fn as_str(&self) -> &'static str {
        match self {
            DeliveryPath::Ws => "ws",
            DeliveryPath::Fcm => "fcm",
            DeliveryPath::Apns => "apns",
            DeliveryPath::Unknown => "unknown",
        }
    }
}

impl EncodeLabelValue for DeliveryPath {
    fn encode(&self, encoder: &mut LabelValueEncoder) -> Result<(), std::fmt::Error> {
        std::fmt::Write::write_str(encoder, self.as_str())
    }
}

/// Whether the notification was read from storage before delivery.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum DeliveryRoute {
    Direct,
    /// Read from storage before delivery, including direct deliveries that
    /// were saved because the client disconnected before ACKing.
    Stored,
}

impl DeliveryRoute {
    pub fn as_str(&self) -> &'static str {
        match self {
            DeliveryRoute::Direct => "direct",
            DeliveryRoute::Stored => "stored",
        }
    }
}

impl EncodeLabelValue for DeliveryRoute {
    fn encode(&self, encoder: &mut LabelValueEncoder) -> Result<(), std::fmt::Error> {
        std::fmt::Write::write_str(encoder, self.as_str())
    }
}

/// Who is responsible for a non-delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    None,
    Client,
    Bridge,
    Push,
}

impl Cause {
    pub fn as_str(&self) -> &'static str {
        match self {
            Cause::None => "none",
            Cause::Client => "client",
            Cause::Bridge => "bridge",
            Cause::Push => "push",
        }
    }
}

/// The terminal status of a notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Received by the client (any ACK code) or accepted by the bridge.
    Delivered,
    /// TTL=0 and not directly deliverable, or a stored message that had
    /// expired by the time it was read. Messages that expire without ever
    /// being read are not counted.
    Expired,
    /// 404, e.g. the endpoint token could not be decrypted.
    NotFound,
    /// 410, the subscription or user no longer exists.
    Gone,
    /// Any other FCM/APNs error.
    BridgeError,
    /// The notification could not be saved to storage.
    StorageError,
    /// Any other 5xx.
    InternalError,
}

impl Outcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Outcome::Delivered => "delivered",
            Outcome::Expired => "expired",
            Outcome::NotFound => "not_found",
            Outcome::Gone => "gone",
            Outcome::BridgeError => "bridge_error",
            Outcome::StorageError => "storage_error",
            Outcome::InternalError => "internal_error",
        }
    }

    pub fn cause(&self) -> Cause {
        match self {
            Outcome::Delivered | Outcome::Expired => Cause::None,
            Outcome::NotFound | Outcome::Gone => Cause::Client,
            Outcome::BridgeError => Cause::Bridge,
            Outcome::StorageError | Outcome::InternalError => Cause::Push,
        }
    }
}

/// Increment `notification.outcome`.
pub fn emit_outcome(metrics: &StatsdClient, path: DeliveryPath, outcome: Outcome) {
    emit_outcomes(metrics, path, outcome, 1);
}

/// Add `count` to `notification.outcome`.
pub fn emit_outcomes(metrics: &StatsdClient, path: DeliveryPath, outcome: Outcome, count: i64) {
    if count == 0 {
        return;
    }
    metrics
        .count_with_tags(MetricName::NotificationOutcome.as_ref(), count)
        .with_tag("path", path.as_str())
        .with_tag("outcome", outcome.as_str())
        .with_tag("cause", outcome.cause().as_str())
        .send();
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct DeliveryLabels {
    path: DeliveryPath,
    route: DeliveryRoute,
}

fn delivery_histogram() -> Histogram {
    Histogram::new(DELIVERY_BUCKETS)
}

/// Seconds elapsed between two millisecond timestamps. Negative durations
/// (cross-host clock skew) are clamped to 0.
fn elapsed_secs(start_ms: u64, end_ms: u64) -> f64 {
    end_ms.saturating_sub(start_ms) as f64 / 1000.0
}

/// The process wide Prometheus registry and the histograms it holds.
#[derive(Debug)]
pub struct DeliveryMetrics {
    registry: Registry,
    delivery_seconds: Family<DeliveryLabels, Histogram, fn() -> Histogram>,
    stored_delivery_after_connect_seconds: Histogram,
    stored_offline_seconds: Histogram,
    /// The legacy reliability state gauges, updated on each `/metrics` request.
    #[cfg(feature = "reliable_report")]
    pub reliability_report: crate::reliability::ReportFamily,
}

impl Default for DeliveryMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl DeliveryMetrics {
    pub fn new() -> Self {
        let mut registry = Registry::default();
        let delivery_seconds =
            Family::<DeliveryLabels, Histogram, fn() -> Histogram>::new_with_constructor(
                delivery_histogram,
            );
        registry.register_with_unit(
            "autopush_delivery",
            "Time from receipt at autoendpoint to delivery",
            Unit::Seconds,
            delivery_seconds.clone(),
        );
        let stored_delivery_after_connect_seconds = delivery_histogram();
        registry.register_with_unit(
            "autopush_stored_delivery_after_connect",
            "Time to deliver a stored message once the client was connected",
            Unit::Seconds,
            stored_delivery_after_connect_seconds.clone(),
        );
        let stored_offline_seconds = delivery_histogram();
        registry.register_with_unit(
            "autopush_stored_offline",
            "Time a stored message waited for the client to connect",
            Unit::Seconds,
            stored_offline_seconds.clone(),
        );
        #[cfg(feature = "reliable_report")]
        let reliability_report = crate::reliability::register_report(&mut registry);
        Self {
            registry,
            delivery_seconds,
            stored_delivery_after_connect_seconds,
            stored_offline_seconds,
            #[cfg(feature = "reliable_report")]
            reliability_report,
        }
    }

    /// Record `autopush_delivery_seconds`.
    pub fn observe_delivery(
        &self,
        path: DeliveryPath,
        route: DeliveryRoute,
        start_ms: u64,
        end_ms: u64,
    ) {
        self.delivery_seconds
            .get_or_create(&DeliveryLabels { path, route })
            .observe(elapsed_secs(start_ms, end_ms));
    }

    /// Record the two parts of a stored message's delivery, split at when it
    /// became deliverable (the later of the client connecting and the message
    /// being received):
    ///
    /// - `autopush_stored_offline_seconds`: receipt to deliverable, the time
    ///   spent waiting for the client to connect.
    /// - `autopush_stored_delivery_after_connect_seconds`: deliverable to ACK.
    pub fn observe_stored_delivery(&self, connected_at_ms: u64, received_ms: u64, ack_ms: u64) {
        let deliverable_ms = connected_at_ms.max(received_ms);
        self.stored_offline_seconds
            .observe(elapsed_secs(received_ms, deliverable_ms));
        self.stored_delivery_after_connect_seconds
            .observe(elapsed_secs(deliverable_ms, ack_ms));
    }

    /// Encode the registry in the OpenMetrics text format.
    pub fn encode(&self) -> Result<String, std::fmt::Error> {
        let mut encoded = String::new();
        encode(&mut encoded, &self.registry)?;
        Ok(encoded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_are_sorted() {
        assert!(DELIVERY_BUCKETS.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(
            *DELIVERY_BUCKETS.last().unwrap(),
            crate::MAX_NOTIFICATION_TTL_SECS as f64
        );
    }

    #[test]
    fn clock_skew_clamped() {
        assert_eq!(elapsed_secs(2_000, 1_000), 0.0);
        assert_eq!(elapsed_secs(1_000, 3_500), 2.5);
    }

    #[test]
    fn encode_report() {
        let metrics = DeliveryMetrics::new();
        metrics.observe_delivery(DeliveryPath::Ws, DeliveryRoute::Direct, 1_000, 1_030);
        metrics.observe_delivery(DeliveryPath::Fcm, DeliveryRoute::Direct, 2_000, 1_000);
        metrics.observe_stored_delivery(10_000, 5_000, 12_000);
        let report = metrics.encode().unwrap();
        assert!(report.contains("# TYPE autopush_delivery_seconds histogram"));
        assert!(report.contains("# UNIT autopush_delivery_seconds seconds"));
        assert!(
            report.contains(
                r#"autopush_delivery_seconds_bucket{le="0.05",path="ws",route="direct"} 1"#
            )
        );
        assert!(report.contains(
            r#"autopush_delivery_seconds_bucket{le="0.01",path="fcm",route="direct"} 1"#
        ));
        assert!(report.contains(r#"autopush_delivery_seconds_count{path="ws",route="direct"} 1"#));
        // 12s - max(10s, 5s) = 2s
        assert!(report.contains("autopush_stored_delivery_after_connect_seconds_sum 2.0"));
        // max(10s, 5s) - 5s = 5s
        assert!(report.contains("autopush_stored_offline_seconds_sum 5.0"));
        assert!(report.trim_end().ends_with("# EOF"));
    }

    #[test]
    fn outcome_causes() {
        assert_eq!(Outcome::Delivered.cause(), Cause::None);
        assert_eq!(Outcome::Expired.cause(), Cause::None);
        assert_eq!(Outcome::NotFound.cause(), Cause::Client);
        assert_eq!(Outcome::Gone.cause(), Cause::Client);
        assert_eq!(Outcome::BridgeError.cause(), Cause::Bridge);
        assert_eq!(Outcome::StorageError.cause(), Cause::Push);
        assert_eq!(Outcome::InternalError.cause(), Cause::Push);
    }
}
