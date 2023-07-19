use std::{
    cell::RefCell,
    rc::Rc,
    sync::Arc,
    task::{Context, Poll},
};

use actix_web::{
    dev::{Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpMessage,
};
use cadence::{CountedExt, StatsdClient};
use futures::{future::LocalBoxFuture, FutureExt};
use futures_util::future::{ok, Ready};
use sentry::{protocol::Event, Hub};

//use crate::LocalError;
use crate::errors::ReportableError;
pub type LocalError = crate::errors::ApcError;
use crate::tags::Tags;

#[derive(Clone)]
pub struct SentryWrapper<E> {
    metrics: Arc<StatsdClient>,
    metric_label: String,
    phantom: std::marker::PhantomData<E>,
}

impl<E> SentryWrapper<E> {
    pub fn new(metrics: Arc<StatsdClient>, metric_label: String) -> Self {
        Self {
            metrics,
            metric_label,
            phantom: std::marker::PhantomData,
        }
    }
}

impl<S, B, E> Transform<S, ServiceRequest> for SentryWrapper<E>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    E: ReportableError + std::error::Error +  actix_web::ResponseError + 'static
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
            phantom: std::marker::PhantomData,
        })
    }
}

#[derive(Debug)]
pub struct SentryWrapperMiddleware<S, E> {
    service: Rc<RefCell<S>>,
    metrics: Arc<StatsdClient>,
    metric_label: String,
    phantom: std::marker::PhantomData<E>,
}

impl<S, B, E> Service<ServiceRequest> for SentryWrapperMiddleware<S, E>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    E: ReportableError + std::error::Error +  actix_web::ResponseError + 'static
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&self, sreq: ServiceRequest) -> Self::Future {
        // Set up the hub to add request data to events
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
                    if let Some(api_err) = error.as_error::<E>() {
                        // if it's not reportable, and we have access to the metrics, record it as a metric.
                        if !api_err.is_sentry_event() {
                            // The error (e.g. VapidErrorKind::InvalidKey(String)) might be too cardinal,
                            // but we may need that information to debug a production issue. We can
                            // add an info here, temporarily turn on info level debugging on a given server,
                            // capture it, and then turn it off before we run out of money.
                            if let Some(label) = api_err.metric_label() {
                                info!("Sentry: Sending error to metrics: {:?}", api_err);
                                let _ = metrics.incr(&format!("{}.{}", metric_label, label));
                            }
                        }
                        debug!("Sentry: Not reporting error (service error): {:?}", error);
                        return Err(error);
                    };
                    debug!("Reporting error to Sentry (service error): {}", error);
                    let mut event = event_from_actix_error::<E>(&error);
                    event.extra.append(&mut tags.clone().extra_tree());
                    event.tags.append(&mut tags.clone().tag_tree());
                    let event_id = hub.capture_event(event);
                    trace!("event_id = {}", event_id);
                    return Err(error);
                }
            };
            // Check for errors inside the response
            if let Some(error) = response.response().error() {
                if let Some(api_err) = error.as_error::<E>() {
                    if !api_err.is_sentry_event() {
                        if let Some(label) = api_err.metric_label() {
                            info!("Sentry: Sending error to metrics: {:?}", api_err);
                            let _ = metrics.incr(&format!("{}.{}", metric_label, label));
                        }
                        debug!("Not reporting error (service error): {:?}", error);
                        return Ok(response);
                    }
                }
                debug!("Reporting error to Sentry (response error): {}", error);
                let mut event = event_from_actix_error::<E>(error);
                event.extra.append(&mut tags.clone().extra_tree());
                event.tags.append(&mut tags.clone().tag_tree());
                let event_id = hub.capture_event(event);
                trace!("event_id = {}", event_id);
            }
            Ok(response)
        }
        .boxed_local()
    }
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

/// Convert Actix errors into a Sentry event. ApiError is handled explicitly so
/// the event can include a backtrace and source error information.
fn event_from_actix_error<E>(error: &actix_web::Error) -> sentry::protocol::Event<'static>
    where 
    E: ReportableError + std::error::Error + actix_web::ResponseError + 'static
{
    // Actix errors don't have support source/cause, so to get more information
    // about the error we need to downcast.
    if let Some(error) = error.as_error::<E>() {
        // Use our error and associated backtrace for the event
        //let mut event = sentry::event_from_error(&error.kind);
        let mut event = error.to_sentry_event();
        event.exception.last_mut().unwrap().stacktrace =
            sentry::integrations::backtrace::backtrace_to_stacktrace(&error.backtrace());
        event
    } else {
        // Fallback to the Actix error
        sentry::event_from_error(error)
    }
}
