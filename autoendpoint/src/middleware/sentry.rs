use crate::error::ApiError;
use crate::tags::Tags;
use actix_web::dev::{Service, ServiceRequest, ServiceResponse};
use cadence::CountedExt;
use sentry::protocol::Event;
use sentry::Hub;
use std::future::Future;

/// Sentry Actix middleware which reports errors to Sentry and includes request
/// information in events.
pub fn sentry_middleware(
    request: ServiceRequest,
    service: &mut impl Service<
        Request = ServiceRequest,
        Response = ServiceResponse,
        Error = actix_web::Error,
    >,
) -> impl Future<Output = Result<ServiceResponse, actix_web::Error>> {
    // Set up the hub to add request data to events
    let hub = Hub::new_from_top(Hub::main());
    let _scope_guard = hub.push_scope();
    let sentry_request = sentry_request_from_http(&request);
    hub.configure_scope(|scope| {
        scope.add_event_processor(Box::new(move |event| process_event(event, &sentry_request)))
    });
    let tags = Tags::from_request_head(request.head());
    let state = request
        .app_data::<actix_web::web::Data<crate::server::ServerState>>()
        .cloned();
    let response = service.call(request);
    async move {
        // Wait for the response and check for errors
        let response: ServiceResponse = match response.await {
            Ok(response) => response,
            Err(error) => {
                if let Some(api_err) = error.as_error::<ApiError>() {
                    // if it's not reportable, and we have access to the metrics, record it as a metric.
                    if !api_err.kind.is_sentry_event() {
                        // The error (e.g. VapidErrorKind::InvalidKey(String)) might be too cardinal,
                        // but we may need that information to debug a production issue. We can
                        // add an info here, temporarily turn on info level debugging on a given server,
                        // capture it, and then turn it off before we run out of money.
                        info!("Sending error to metrics: {:?}", api_err.kind);
                        if let Some(state) = state {
                            match state
                                .metrics
                                .incr(&format!("api_error.{}", api_err.kind.metric_label()))
                            {
                                Ok(_) | Err(_) => {}
                            };
                        }
                        debug!("Not reporting error (service error): {:?}", error);
                        return Err(error);
                    }
                }
                debug!("Reporting error to Sentry (service error): {}", error);
                let mut event = event_from_actix_error(&error);
                event.extra = tags.extra_tree();
                let event_id = hub.capture_event(event);
                trace!("event_id = {}", event_id);
                return Err(error);
            }
        };

        // Check for errors inside the response
        if let Some(error) = response.response().error() {
            if let Some(api_err) = error.as_error::<ApiError>() {
                if !api_err.kind.is_sentry_event() {
                    debug!("Not reporting error (service error): {:?}", error);
                    drop(_scope_guard);
                    return Ok(response);
                }
            }
            debug!("Reporting error to Sentry (response error): {}", error);
            let event_id = hub.capture_event(event_from_actix_error(error));
            trace!("event_id = {}", event_id);
        }

        // Move the guard into the future and keep it from dropping until now
        drop(_scope_guard);

        Ok(response)
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
fn event_from_actix_error(error: &actix_web::Error) -> sentry::protocol::Event<'static> {
    // Actix errors don't have support source/cause, so to get more information
    // about the error we need to downcast.
    if let Some(error) = error.as_error::<ApiError>() {
        // Use our error and associated backtrace for the event
        let mut event = sentry::event_from_error(&error.kind);
        event.exception.last_mut().unwrap().stacktrace =
            sentry::integrations::backtrace::backtrace_to_stacktrace(&error.backtrace);
        event
    } else {
        // Fallback to the Actix error
        sentry::event_from_error(error)
    }
}
