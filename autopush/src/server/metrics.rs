//! Metrics tie-ins

use std::net::UdpSocket;

use cadence::{BufferedUdpMetricSink, NopMetricSink, QueuingMetricSink, StatsdClient};

use autopush_common::errors::Result;

use crate::server::ServerOptions;

/// Create a cadence StatsdClient from the given options
pub fn metrics_from_opts(opts: &ServerOptions) -> Result<StatsdClient> {
    let builder = if let Some(statsd_host) = opts.statsd_host.as_ref() {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.set_nonblocking(true)?;

        let host = (statsd_host.as_str(), opts.statsd_port);
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
