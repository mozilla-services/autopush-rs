use std::net::UdpSocket;

use cadence::{
    BufferedUdpMetricSink, Counted, Counter, MetricBuilder, MetricError, MetricResult,
    NopMetricSink, QueuingMetricSink, StatsdClient, StatsdClientBuilder,
};

use crate::metric_name::MetricName;

/// Extension trait for StatsdClient to provide enum-based metric methods
pub trait StatsdClientExt {
    /// Increment a counter using a MetricName enum
    fn incr(&self, metric: MetricName) -> MetricResult<Counter>;

    /// Increment a counter using a raw string metric name
    fn incr_raw(&self, metric: &str) -> MetricResult<Counter>;

    /// Start a counter with tags using a MetricName enum
    fn incr_with_tags(&self, metric: MetricName) -> MetricBuilder<'_, '_, Counter>;
}

impl StatsdClientExt for StatsdClient {
    fn incr(&self, metric: MetricName) -> MetricResult<Counter> {
        let metric_tag: &'static str = metric.into();
        self.count(metric_tag, 1)
    }

    fn incr_raw(&self, metric: &str) -> MetricResult<Counter> {
        self.count(metric, 1)
    }

    fn incr_with_tags(&self, metric: MetricName) -> MetricBuilder<'_, '_, Counter> {
        let metric_tag: &'static str = metric.into();
        self.count_with_tags(metric_tag, 1)
    }
}

/// Create a cadence StatsdClientBuilder from the given options
pub fn builder(
    prefix: &str,
    host: &Option<String>,
    port: u16,
) -> Result<StatsdClientBuilder, MetricError> {
    let builder = if let Some(host) = host {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.set_nonblocking(true)?;

        let addr = (host.as_str(), port);
        let udp_sink = BufferedUdpMetricSink::from(addr, socket)?;
        let sink = QueuingMetricSink::from(udp_sink);
        StatsdClient::builder(prefix, sink)
    } else {
        StatsdClient::builder(prefix, NopMetricSink)
    };
    Ok(builder.with_error_handler(|err| warn!("⚠️ Metric send error: {:?}", err)))
}
