use autoconnect_ws_sm::WebPushClient;

use crate::error::WSError;

pub fn capture_error(client: &WebPushClient, err: &WSError) {
    let mut event = sentry::event_from_error(err);
    // TODO:
    //event.exception.last_mut().unwrap().stacktrace =
    //    sentry::integrations::backtrace::backtrace_to_stacktrace(&err.backtrace);

    event.user = Some(sentry::User {
        id: Some(client.uaid.as_simple().to_string()),
        ..Default::default()
    });
    let ua_info = client.ua_info.clone();
    event
        .tags
        .insert("ua_name".to_owned(), ua_info.browser_name);
    event
        .tags
        .insert("ua_os_family".to_owned(), ua_info.metrics_os);
    event
        .tags
        .insert("ua_os_ver".to_owned(), ua_info.os_version);
    event
        .tags
        .insert("ua_browser_family".to_owned(), ua_info.metrics_browser);
    event
        .tags
        .insert("ua_browser_ver".to_owned(), ua_info.browser_version);
    sentry::capture_event(event);
}
