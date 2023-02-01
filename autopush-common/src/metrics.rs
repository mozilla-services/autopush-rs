//! Metrics tie-ins

use std::net::UdpSocket;

use crate::errors::Result;
use cadence::{BufferedUdpMetricSink, NopMetricSink, QueuingMetricSink, StatsdClient};

/// Create a cadence StatsdClient from the given options
pub fn new_metrics(host: Option<String>, port: u16) -> Result<StatsdClient> {
    let builder = if let Some(statsd_host) = host.as_ref() {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.set_nonblocking(true)?;

        let host = (statsd_host.as_str(), port);
        let udp_sink = BufferedUdpMetricSink::from(host, socket)?;
        let sink = QueuingMetricSink::from(udp_sink);
        StatsdClient::builder("autopush", sink)
    } else {
        StatsdClient::builder("autopush", NopMetricSink)
    };
    Ok(builder
        .with_error_handler(|err| error!("Metrics send error: {}", err))
        .build())
}
