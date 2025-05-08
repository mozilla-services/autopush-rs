use std::{sync::Arc, time::Instant};

use actix_web::{dev::Payload, web::Data, FromRequest, HttpMessage, HttpRequest};
use cadence::{Metric, MetricError, NopMetricSink, StatsdClient, Timed};
use futures::future;

use autopush_common::{metric_name::MetricName, metrics::StatsdClientExt, tags::Tags};

use crate::{error::ApiError, server::AppState, settings::Settings};

#[derive(Debug, Clone)]
pub struct MetricTimer {
    pub label: String,
    pub start: Instant,
    pub tags: Tags,
}

#[derive(Debug, Clone)]
pub struct Metrics {
    client: Option<Arc<StatsdClient>>,
    timer: Option<MetricTimer>,
    tags: Option<Tags>,
}

impl Drop for Metrics {
    fn drop(&mut self) {
        let tags = self.tags.clone().unwrap_or_default();
        if let Some(client) = self.client.as_ref() {
            if let Some(timer) = self.timer.as_ref() {
                let lapse = (Instant::now() - timer.start).as_millis() as u64;
                trace!(
                    "⌚ Ending timer at nanos: {:?} : {:?}",
                    &timer.label,
                    lapse;
                    tags
                );
                let mut tagged = client.time_with_tags(&timer.label, lapse);
                // Include any "hard coded" tags.
                // tagged = tagged.with_tag("version", env!("CARGO_PKG_VERSION"));
                let tags = timer.tags.tags.clone();
                let keys = tags.keys();
                for tag in keys {
                    tagged = tagged.with_tag(tag, tags.get(tag).unwrap())
                }
                match tagged.try_send() {
                    Err(e) => {
                        // eat the metric, but log the error
                        warn!("⚠️ Metric {} error: {:?} ", &timer.label, e);
                    }
                    Ok(v) => {
                        trace!("⌚ {:?}", v.as_metric_str());
                    }
                }
            }
        }
    }
}

impl FromRequest for Metrics {
    type Error = ApiError;
    type Future = future::Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        future::ok(Metrics::from(req))
    }
}

impl From<&HttpRequest> for Metrics {
    fn from(req: &HttpRequest) -> Self {
        let exts = req.extensions();
        let def_tags = Tags::from_request_head(req.head());
        let tags = exts.get::<Tags>().unwrap_or(&def_tags);
        Metrics {
            client: Some(metrics_from_req(req)),
            tags: Some(tags.clone()),
            timer: None,
        }
    }
}

impl From<StatsdClient> for Metrics {
    fn from(client: StatsdClient) -> Self {
        Metrics {
            client: Some(Arc::new(client)),
            tags: None,
            timer: None,
        }
    }
}

impl From<&actix_web::web::Data<AppState>> for Metrics {
    fn from(state: &actix_web::web::Data<AppState>) -> Self {
        Metrics {
            client: Some(state.metrics.clone()),
            tags: None,
            timer: None,
        }
    }
}

impl Metrics {
    #![allow(unused)] // TODO: Start using metrics

    pub fn sink() -> StatsdClient {
        StatsdClient::builder("", NopMetricSink).build()
    }

    pub fn noop() -> Self {
        Self {
            client: Some(Arc::new(Self::sink())),
            timer: None,
            tags: None,
        }
    }

    pub fn start_timer(&mut self, label: &str, tags: Option<Tags>) {
        let mut mtags = self.tags.clone().unwrap_or_default();
        if let Some(t) = tags {
            mtags.extend(t.tags)
        }

        trace!("⌚ Starting timer... {:?}", &label; &mtags);
        self.timer = Some(MetricTimer {
            label: label.to_owned(),
            start: Instant::now(),
            tags: mtags,
        });
    }

    // increment a counter with no tags data.
    pub fn incr(self, metric: MetricName, tags: Option<Tags>) {
        self.incr_with_tags(metric, tags)
    }

    pub fn incr_with_tags(self, metric: MetricName, tags: Option<Tags>) {
        if let Some(client) = self.client.as_ref() {
            let mut tagged = client.incr_with_tags(metric.clone());
            let mut mtags = self.tags.clone().unwrap_or_default();
            if let Some(t) = tags {
                mtags.tags.extend(t.tags)
            }
            let tag_keys = mtags.tags.keys();
            for key in tag_keys.clone() {
                tagged = tagged.with_tag(key, mtags.tags.get(key).unwrap());
            }
            match tagged.try_send() {
                Err(e) => {
                    // eat the metric, but log the error
                    warn!("⚠️ Metric {} error: {:?}", metric.as_ref(), e; mtags);
                }
                Ok(v) => trace!("☑️ {:?}", v.as_metric_str()),
            }
        }
    }
}

pub fn metrics_from_req(req: &HttpRequest) -> Arc<StatsdClient> {
    req.app_data::<Data<AppState>>()
        .expect("Could not get state in metrics_from_req")
        .metrics
        .clone()
}

/// Create a cadence StatsdClient from the given options
pub fn metrics_from_settings(settings: &Settings) -> Result<StatsdClient, MetricError> {
    let client = autopush_common::metrics::builder(
        &settings.statsd_label,
        &settings.statsd_host,
        settings.statsd_port,
    )?
    .build();
    Ok(client)
}
