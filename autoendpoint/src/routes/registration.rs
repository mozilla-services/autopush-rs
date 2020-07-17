use crate::auth::sign_with_key;
use crate::error::{ApiErrorKind, ApiResult};
use crate::extractors::authorization_check::AuthorizationCheck;
use crate::extractors::registration_path_args::RegistrationPathArgs;
use crate::extractors::router_data_input::RouterDataInput;
use crate::extractors::routers::Routers;
use crate::headers::util::get_header;
use crate::server::ServerState;
use actix_web::web::Data;
use actix_web::{HttpRequest, HttpResponse};
use autopush_common::db::DynamoDbUser;
use autopush_common::endpoint::make_endpoint;
use cadence::{Counted, StatsdClient};
use uuid::Uuid;

/// Handle the `POST /v1/{router_type}/{app_id}/registration` route
pub async fn register_uaid_route(
    path_args: RegistrationPathArgs,
    router_data_input: RouterDataInput,
    routers: Routers,
    state: Data<ServerState>,
    request: HttpRequest,
) -> ApiResult<HttpResponse> {
    // Register with router
    debug!(
        "Registering a user with the {} router",
        path_args.router_type
    );
    trace!("token = {}", router_data_input.token);
    let router = routers.get(path_args.router_type);
    let router_data = router.register(&router_data_input, &path_args.app_id)?;
    incr_metric("ua.command.register", &state.metrics, &request);

    // Register user and channel in database
    let user = DynamoDbUser {
        router_type: path_args.router_type.to_string(),
        router_data: Some(router_data),
        current_month: Some(state.ddb.message_table().to_string()),
        ..Default::default()
    };
    let channel_id = router_data_input.channel_id.unwrap_or_else(Uuid::new_v4);
    trace!("Creating user with UAID {}", user.uaid);
    trace!("user = {:?}", user);
    trace!("channel_id = {}", channel_id);
    state.ddb.add_user(&user).await?;
    state.ddb.add_channel(user.uaid, channel_id).await?;

    // Make the endpoint URL
    trace!("Creating endpoint for user");
    let endpoint_url = make_endpoint(
        &user.uaid,
        &channel_id,
        router_data_input.key.as_deref(),
        state.settings.endpoint_url.as_str(),
        &state.fernet,
    )
    .map_err(ApiErrorKind::EndpointUrl)?;
    trace!("endpoint = {}", endpoint_url);

    // Create the secret
    trace!("Creating secret for UAID {}", user.uaid);
    let auth_keys = state.settings.auth_keys();
    let auth_key = auth_keys
        .get(0)
        .expect("At least one auth key must be provided in the settings");
    let secret = sign_with_key(auth_key.as_bytes(), user.uaid.as_bytes())
        .map_err(ApiErrorKind::RegistrationSecretHash)?;

    trace!("Finished registering UAID {}", user.uaid);
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "uaid": user.uaid,
        "channelID": channel_id,
        "endpoint": endpoint_url,
        "secret": secret
    })))
}

/// Handle the `PUT /v1/{router_type}/{app_id}/registration/{uaid}` route
pub async fn update_token_route(
    _auth: AuthorizationCheck,
    path_args: RegistrationPathArgs,
    router_data_input: RouterDataInput,
    routers: Routers,
    state: Data<ServerState>,
    request: HttpRequest,
) -> ApiResult<HttpResponse> {
    unimplemented!()
}

/// Increment a metric with data from the request
fn incr_metric(name: &str, metrics: &StatsdClient, request: &HttpRequest) {
    metrics
        .incr_with_tags(name)
        .with_tag(
            "user_agent",
            get_header(&request, "User-Agent").unwrap_or("unknown"),
        )
        .with_tag("host", get_header(&request, "Host").unwrap_or("unknown"))
        .send()
}
