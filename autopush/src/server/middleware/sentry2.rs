/// TODO: Eventually move this to autopush-common, as well as handle a number of the
/// issues with modern sentry:
/// e.g.
///     * resolve bytes limit
///     * handle pulling `extra` data
use std::borrow::Cow;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::http::StatusCode;
use actix_web::Error;
use futures_util::future::{ok, Future, Ready};
use futures_util::FutureExt;

use sentry_core::protocol::{self, ClientSdkPackage, Event, Request};
use sentry_core::{Hub, SentryFutureExt};

// Taken from sentry-actix

/// Reports certain failures to Sentry.
#[derive(Clone)]
pub struct Sentry {
    hub: Option<Arc<Hub>>,
    emit_header: bool,
    capture_server_errors: bool,
    start_transaction: bool,
}

impl Default for Sentry {
    fn default() -> Self {
        Sentry {
            hub: None,
            emit_header: false,
            capture_server_errors: true,
            start_transaction: false,
        }
    }
}

impl Sentry {
    #[allow(dead_code)]
    /// Creates a new sentry middleware which starts a new performance monitoring transaction for each request.
    pub fn with_transaction() -> Sentry {
        Sentry {
            start_transaction: true,
            ..Sentry::default()
        }
    }
}

impl<S, B> Transform<S, ServiceRequest> for Sentry
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = SentryMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(SentryMiddleware {
            service,
            inner: self.clone(),
        })
    }
}

/// The middleware for individual services.
pub struct SentryMiddleware<S> {
    service: S,
    inner: Sentry,
}

impl<S, B> Service<ServiceRequest> for SentryMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(
        &self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let inner = self.inner.clone();
        // XXX Sentry Modification
        // crate::server is going to be specific to the given app, so we can't put this in common.
        let state = req
            .app_data::<actix_web::web::Data<crate::server::ServerState>>()
            .cloned();
        // XXX
        let hub = Arc::new(Hub::new_from_top(
            inner.hub.clone().unwrap_or_else(Hub::main),
        ));
        let client = hub.client();
        let track_sessions = client.as_ref().map_or(false, |client| {
            let options = client.options();
            options.auto_session_tracking
                && options.session_mode == sentry_core::SessionMode::Request
        });
        if track_sessions {
            hub.start_session();
        }
        let with_pii = client
            .as_ref()
            .map_or(false, |client| client.options().send_default_pii);

        let (mut tx, sentry_req) = sentry_request_from_http(&req, with_pii);

        let transaction = if inner.start_transaction {
            let name = std::mem::take(&mut tx)
                .unwrap_or_else(|| format!("{} {}", req.method(), req.uri()));

            let headers = req.headers().iter().flat_map(|(header, value)| {
                value.to_str().ok().map(|value| (header.as_str(), value))
            });

            // TODO: Break apart user agent?
            // XXX Sentry modification
            /*
            if let Some(ua_val) = req.headers().get("UserAgent") {
                if let Ok(ua_str) = ua_val.to_str() {
                    let ua_info = crate::util::user_agent::UserAgentInfo::from(ua_val.to_str().unwrap_or_default());
                    sentry::configure_scope(|scope| {
                        scope::add_tag("browser_version", ua_info.browser_version);
                    })
                }
            }
            */
            let ctx = sentry_core::TransactionContext::continue_from_headers(
                &name,
                "http.server",
                headers,
            );
            Some(hub.start_transaction(ctx))
        } else {
            None
        };

        let parent_span = hub.configure_scope(|scope| {
            let parent_span = scope.get_span();
            if let Some(transaction) = transaction.as_ref() {
                scope.set_span(Some(transaction.clone().into()));
            } else {
                scope.set_transaction(tx.as_deref());
            }
            scope.add_event_processor(move |event| Some(process_event(event, &sentry_req)));
            parent_span
        });

        let fut = self.service.call(req).bind_hub(hub.clone());

        async move {
            // Service errors
            let mut res: Self::Response = match fut.await {
                Ok(res) => res,
                Err(e) => {
                    // XXX Sentry Modification
                    if let Some(api_err) = e.as_error::<crate::errors::ApcError>() {
                        // if it's not reportable, , and we have access to the metrics, record it as a metric.
                        if !api_err.kind.is_sentry_event() {
                            // XXX - Modified sentry
                            // The error (e.g. VapidErrorKind::InvalidKey(String)) might be too cardinal,
                            // but we may need that information to debug a production issue. We can
                            // add an info here, temporarily turn on info level debugging on a given server,
                            // capture it, and then turn it off before we run out of money.
                            info!("Sending error to metrics: {:?}", api_err.kind);
                            if let Some(state) = state {
                                if let Some(label) = api_err.kind.metric_label() {
                                    state.metrics.incr(&format!("api_error.{}", label)).is_ok();
                                };
                            }
                            debug!("Not reporting error (service error): {:?}", e);
                            return Err(e);
                        }
                    }

                    if inner.capture_server_errors {
                        hub.capture_error(&e);
                    }

                    if let Some(transaction) = transaction {
                        if transaction.get_status().is_none() {
                            let status = protocol::SpanStatus::UnknownError;
                            transaction.set_status(status);
                        }
                        transaction.finish();
                        hub.configure_scope(|scope| scope.set_span(parent_span));
                    }
                    return Err(e);
                }
            };

            // Response errors
            if inner.capture_server_errors && res.response().status().is_server_error() {
                if let Some(e) = res.response().error() {
                    let event_id = hub.capture_error(e);

                    if inner.emit_header {
                        res.response_mut().headers_mut().insert(
                            "x-sentry-event".parse().unwrap(),
                            event_id.simple().to_string().parse().unwrap(),
                        );
                    }
                }
            }

            if let Some(transaction) = transaction {
                if transaction.get_status().is_none() {
                    let status = map_status(res.status());
                    transaction.set_status(status);
                }
                transaction.finish();
                hub.configure_scope(|scope| scope.set_span(parent_span));
            }

            Ok(res)
        }
        .boxed_local()
    }
}

fn map_status(status: StatusCode) -> protocol::SpanStatus {
    match status {
        StatusCode::UNAUTHORIZED => protocol::SpanStatus::Unauthenticated,
        StatusCode::FORBIDDEN => protocol::SpanStatus::PermissionDenied,
        StatusCode::NOT_FOUND => protocol::SpanStatus::NotFound,
        StatusCode::TOO_MANY_REQUESTS => protocol::SpanStatus::ResourceExhausted,
        status if status.is_client_error() => protocol::SpanStatus::InvalidArgument,
        StatusCode::NOT_IMPLEMENTED => protocol::SpanStatus::Unimplemented,
        StatusCode::SERVICE_UNAVAILABLE => protocol::SpanStatus::Unavailable,
        status if status.is_server_error() => protocol::SpanStatus::InternalError,
        StatusCode::CONFLICT => protocol::SpanStatus::AlreadyExists,
        status if status.is_success() => protocol::SpanStatus::Ok,
        _ => protocol::SpanStatus::UnknownError,
    }
}

/// Build a Sentry request struct from the HTTP request
fn sentry_request_from_http(request: &ServiceRequest, with_pii: bool) -> (Option<String>, Request) {
    let transaction = if let Some(name) = request.match_name() {
        Some(String::from(name))
    } else {
        request.match_pattern()
    };

    let mut sentry_req = Request {
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
    };

    // If PII is enabled, include the remote address
    if with_pii {
        if let Some(remote) = request.connection_info().peer_addr() {
            sentry_req.env.insert("REMOTE_ADDR".into(), remote.into());
        }
    };

    (transaction, sentry_req)
}

/// Add request data to a Sentry event
fn process_event(mut event: Event<'static>, request: &Request) -> Event<'static> {
    // Request
    if event.request.is_none() {
        event.request = Some(request.clone());
    }

    // SDK
    if let Some(sdk) = event.sdk.take() {
        let mut sdk = sdk.into_owned();
        sdk.packages.push(ClientSdkPackage {
            name: "sentry-actix".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        });
        event.sdk = Some(Cow::Owned(sdk));
    }
    event
}
