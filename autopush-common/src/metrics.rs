//! Metrics tie-ins
use std::net::UdpSocket;

use cadence::{
    BufferedUdpMetricSink, MetricError, NopMetricSink, QueuingMetricSink, StatsdClient,
    StatsdClientBuilder,
};

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
