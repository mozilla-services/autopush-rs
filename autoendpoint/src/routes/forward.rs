use actix_http::StatusCode;
use actix_web::{
    HttpResponse, HttpResponseBuilder,
    web::{Bytes, Data},
};

use crate::{
    error::{ApiError, ApiErrorKind, ApiResult},
    extractors::{forward_request::ForwardRequest, forward_token::ForwardToken},
    routers::RouterError,
    server::AppState,
};

/// Forward request to the endpoint stored in the encrypted token
pub async fn forward_route(
    forward_req: ForwardRequest,
    data: Bytes,
    _app_state: Data<AppState>,
) -> ApiResult<HttpResponse> {
    if data.is_empty() {
        return Ok(HttpResponse::Ok().finish());
    }
    let data_len = data.len();
    if data_len > 4096 {
        return Err(ApiError::from(RouterError::TooMuchData(data_len)));
    }
    let ret = forward_req
        .0
        .body(data)
        .send()
        .await
        .map_err(|e| e.without_url())?;
    Ok(HttpResponseBuilder::new(
        // we could probably unwrap, as from_u16 can't have an invalid status code
        StatusCode::from_u16(ret.status().as_u16())
            .map_err(|_| ApiErrorKind::General("Unknown status code".into()))?,
    )
    .json(serde_json::json!({"msg": "upstream status code"})))
}

/// Creates an endpoint to forward notifications to another webpush server
///
/// The request body is a JSON deserialized to [ForwardEndpointRequest], containing:
/// - url: URL to forward notifs to, must be HTTPS
/// - server_pubkey: (optional) VAPID pubkey of the application server - uncompressed format URL-safe Base64 encoded
/// - forward_privkey: (optional, requires server_pubkey): VAPID privkey to authorize forwarded requests, PEM format
///
/// **Example:**
///
/// ```json
/// {
///   "url": "https://myowndomain.tld/random1",
///   "server_pubkey": "BA1Hxzyi1RUM1b5wjxsn7nGxAszw2u61m164i3MrAIxHF6YK5h4SDYic-dRuU_RcPcfA5aq9ojSwk5Y2EmclBPs",
///   "forward_privkey": "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgZImOgpRszunnU3j1\noX5UQiX8KU4X2OdbENuvc/t8wpmhRANCAATN21Y1v8LmQueGpSG6o022gTbbYa4l\nbXWZXITsjknW1WHmELtouYpyXX7e41FiAMuDvcRwW2Nfehn/taHW/IXb\n-----END PRIVATE KEY-----"
/// }
/// ```
///
/// The endpoint contains an encrypted token which contains:
/// - the push endpoint
/// - the hash of the application server VAPID public key - or [0u8; 32]
/// - the VAPID private key to authorize the outgoing request - or [0u8; 32]
///
/// The token contains a VAPID private key because:
/// - We want to use VAPID on the receiving server
/// - We can't use the same VAPID key for all outgoint requests, anyone who knows an endpoint
///   authorized to receive push notifications from us would be able to push to that endpoint
/// - So we need a different key for all endpoints
pub async fn new_forward_endpoint_route(
    token: ForwardToken,
    app_state: Data<AppState>,
) -> ApiResult<HttpResponse> {
    let mut root = url::Url::parse(&app_state.settings.endpoint_url)
        .map_err(|e| ApiErrorKind::General(format!("Cannot parse settings endpoint_url: {e}")))?;
    root.set_path(&format!("/fpush/v1/{}", token.0));
    Ok(HttpResponse::Ok().json(serde_json::json!({"url": root})))
}
