use std::{cell::RefCell, marker::PhantomData, rc::Rc, sync::Arc};

use actix_web::{
    dev::{Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpMessage,
};
use cadence::{CountedExt, StatsdClient};
use futures::{future::LocalBoxFuture, FutureExt};
use futures_util::future::{ok, Ready};
use sentry::{protocol::Event, Hub};

use crate::{errors::ReportableError, tags::Tags};

#[derive(Clone)]
pub struct SentryWrapper<E> {
    metrics: Arc<StatsdClient>,
    metric_label: String,
    phantom: PhantomData<E>,
    sentry_disabled: bool,
}

impl<E> SentryWrapper<E> {
    pub fn new(metrics: Arc<StatsdClient>, metric_label: String, sentry_disabled: bool) -> Self {
        Self {
            metrics,
            metric_label,
            phantom: PhantomData,
            sentry_disabled,
        }
    }
}

impl<S, B, E> Transform<S, ServiceRequest> for SentryWrapper<E>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    E: ReportableError + actix_web::ResponseError + 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = SentryWrapperMiddleware<S, E>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(SentryWrapperMiddleware {
            service: Rc::new(RefCell::new(service)),
            metrics: self.metrics.clone(),
            metric_label: self.metric_label.clone(),
            phantom: PhantomData,
            sentry_disabled: self.sentry_disabled,
        })
    }
}

#[derive(Debug)]
pub struct SentryWrapperMiddleware<S, E> {
    service: Rc<RefCell<S>>,
    metrics: Arc<StatsdClient>,
    metric_label: String,
    phantom: PhantomData<E>,
    sentry_disabled: bool,
}

impl<S, B, E> Service<ServiceRequest> for SentryWrapperMiddleware<S, E>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    E: ReportableError + actix_web::ResponseError + 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    actix_web::dev::forward_ready!(service);

    /// Set up the hub to add request data to events
    fn call(&self, sreq: ServiceRequest) -> Self::Future {
        let hub = Hub::new_from_top(Hub::main());
        let _ = hub.push_scope();
        let sentry_request = sentry_request_from_http(&sreq);
        hub.configure_scope(|scope| {
            scope.add_event_processor(Box::new(move |event| process_event(event, &sentry_request)))
        });

        // get the tag information
        let mut tags = Tags::from_request_head(sreq.head());
        let metrics = self.metrics.clone();
        let metric_label = self.metric_label.clone();
        let sentry_disabled = self.sentry_disabled;
        if let Some(rtags) = sreq.request().extensions().get::<Tags>() {
            trace!("Sentry: found tags in request: {:?}", &rtags.tags);
            for (k, v) in rtags.tags.clone() {
                tags.tags.insert(k, v);
            }
        };
        sreq.extensions_mut().insert(tags.clone());

        let fut = self.service.call(sreq);

        async move {
            let response: Self::Response = match fut.await {
                Ok(response) => response,
                Err(error) => {
                    if let Some(reportable_err) = error.as_error::<E>() {
                        // if it's not reportable, and we have access to the metrics, record it as a metric.
                        if !reportable_err.is_sentry_event() {
                            // The error (e.g. VapidErrorKind::InvalidKey(String)) might be too cardinal,
                            // but we may need that information to debug a production issue. We can
                            // add an info here, temporarily turn on info level debugging on a given server,
                            // capture it, and then turn it off before we run out of money.
                            maybe_emit_metrics(&metrics, &metric_label, reportable_err);
                            debug!("Sentry: Not reporting error (service error): {:?}", error);
                            return Err(error);
                        }
                    };
                    debug!("Reporting error to Sentry (service error): {}", error);
                    let event = event_from_actix_error::<E>(&error);
                    if sentry_disabled {
                        log_event(&event);
                    } else {
                        let event_id = hub.capture_event(event);
                        trace!("event_id = {}", event_id);
                    }
                    return Err(error);
                }
            };
            // Check for errors inside the response
            if let Some(error) = response.response().error() {
                if let Some(reportable_err) = error.as_error::<E>() {
                    if !reportable_err.is_sentry_event() {
                        maybe_emit_metrics(&metrics, &metric_label, reportable_err);
                        debug!("Not reporting error (service error): {:?}", error);
                        return Ok(response);
                    }
                }
                debug!("Reporting error to Sentry (response error): {}", error);
                let event = event_from_actix_error::<E>(error);
                let event_id = hub.capture_event(event);
                trace!("event_id = {}", event_id);
            }
            Ok(response)
        }
        .boxed_local()
    }
}

/// Log the Sentry event to STDERR. (This is copied from syncstorage-rs)
fn log_event(event: &Event) {
    let exception = event.exception.last();
    let error_type = exception.map_or("UnknownError", |e| e.ty.as_str());
    let error_value = exception
        .and_then(|e| e.value.as_deref())
        .unwrap_or("No error message");
    error!("{}", error_value;
        "error_type" => error_type,
        "tags" => ?event.tags,
        "extra" => ?event.extra,
        "url" => event.request.as_ref().and_then(|r| r.url.as_ref()).map(|u| u.to_string()).unwrap_or_default(),
        "method" => event.request.as_ref().and_then(|r| r.method.as_deref()).unwrap_or_default(),
        "stacktrace" => exception.and_then(|e| e.stacktrace.as_ref()).map(|s| format!("{:?}", s.frames)).unwrap_or_default(),
    );
}

/// Emit metrics when a [ReportableError::metric_label] is returned
fn maybe_emit_metrics<E>(metrics: &StatsdClient, label_prefix: &str, err: &E)
where
    E: ReportableError,
{
    let Some(label) = err.metric_label() else {
        return;
    };
    debug!("Sending error to metrics: {:?}", err);
    let label = format!("{label_prefix}.{label}");
    let mut builder = metrics.incr_with_tags(&label);
    let tags = err.tags();
    for (key, val) in &tags {
        builder = builder.with_tag(key, val);
    }
    builder.send();
}

/// Build a Sentry request struct from the HTTP request
fn sentry_request_from_http(request: &ServiceRequest) -> sentry::protocol::Request {
    sentry::protocol::Request {
        url: format!(
            "{}://{}{}",
            request.connection_info().scheme(),
            request.connection_info().host(),
            request.uri()
        )
        .parse()
        .ok(),
        method: Some(request.method().to_string()),
        headers: request
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_string()))
            .collect(),
        ..Default::default()
    }
}

/// Add request data to a Sentry event
#[allow(clippy::unnecessary_wraps)]
fn process_event(
    mut event: Event<'static>,
    request: &sentry::protocol::Request,
) -> Option<Event<'static>> {
    if event.request.is_none() {
        event.request = Some(request.clone());
    }

    // TODO: Use ServiceRequest::match_pattern for the event transaction.
    //       Coming in Actix v3.

    Some(event)
}

/// Convert Actix errors into a Sentry event. ReportableError is handled
/// explicitly so the event can include a backtrace and source error
/// information.
fn event_from_actix_error<E>(error: &actix_web::Error) -> sentry::protocol::Event<'static>
where
    E: ReportableError + actix_web::ResponseError + 'static,
{
    // Actix errors don't have support source/cause, so to get more information
    // about the error we need to downcast.
    if let Some(reportable_err) = error.as_error::<E>() {
        // Use our error and associated backtrace for the event
        crate::sentry::event_from_error(reportable_err)
    } else {
        // Fallback to the Actix error
        sentry::event_from_error(error)
    }
}
