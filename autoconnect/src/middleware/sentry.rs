use std::{cell::RefCell, rc::Rc, task::Context};

use actix_web::{
    dev::{Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpMessage,
};
use cadence::CountedExt;
use futures::{future::LocalBoxFuture, FutureExt};
use futures_util::future::{ok, Ready};
use sentry::protocol::Event;
use std::task::Poll;

use autopush_common::errors::ApcError;
use autopush_common::tags::Tags;

#[derive(Default, Clone)]
pub struct SentryWrapper;

impl<S, B> Transform<S, ServiceRequest> for SentryWrapper
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = SentryWrapperMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(SentryWrapperMiddleware {
            service: Rc::new(RefCell::new(service)),
        })
    }
}

#[derive(Debug)]
pub struct SentryWrapperMiddleware<S> {
    service: Rc<RefCell<S>>,
}

/// Report an error to Sentry after applying the `tags`
pub fn report(tags: &Tags, mut event: Event<'static>) {
    let tags = tags.clone();
    event.tags = tags.clone().tag_tree();
    event.extra = tags.extra_tree();
    trace!("Sentry: Sending error: {:?}", &event);
    sentry::capture_event(event);
}

impl<S, B> Service<ServiceRequest> for SentryWrapperMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&self, sreq: ServiceRequest) -> Self::Future {
        let mut tags = Tags::from_request_head(sreq.head());
        if let Some(rtags) = sreq.request().extensions().get::<Tags>() {
            trace!("Sentry: found tags in request: {:?}", &rtags.tags);
            for (k, v) in rtags.tags.clone() {
                tags.tags.insert(k, v);
            }
        };
        // XXX for autoconnect
        let state = sreq
            .app_data::<actix_web::web::Data<autoconnect_settings::options::ServerOptions>>()
            .cloned();

        sreq.extensions_mut().insert(tags.clone());

        let fut = self.service.call(sreq);

        async move {
            let resp: Self::Response = match fut.await {
                Ok(resp) => {
                    if let Some(events) = resp
                        .request()
                        .extensions_mut()
                        .remove::<Vec<Event<'static>>>()
                    {
                        for event in events {
                            trace!("Sentry: found error stored in request: {:?}", &event);
                            report(&tags, event);
                        }
                    };
                    resp
                }
                Err(err) => {
                    if let Some(api_err) = err.as_error::<ApcError>() {
                        // skip reporting error if need be
                        if !api_err.kind.is_sentry_event() {
                            trace!("Sentry: Sending error to metrics: {:?}", api_err);
                            if let Some(state) = state {
                                if let Some(label) = api_err.kind.metric_label() {
                                    let _ = state.metrics.incr(&format!("apc_error.{}", label));
                                }
                            }
                            return Err(err);
                        }
                        // Sentry should capture backtrace and other functions automatically if
                        // the default "backtrace" feature is specified in Cargo.toml
                        report(&tags, sentry::event_from_error(&err));
                    };
                    return Err(err);
                }
            };

            Ok(resp)
        }
        .boxed_local()
    }
}
