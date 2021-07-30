use crate::auth::sign_with_key;
use crate::error::{ApiErrorKind, ApiResult};
use crate::extractors::authorization_check::AuthorizationCheck;
use crate::extractors::new_channel_data::NewChannelData;
use crate::extractors::registration_path_args::RegistrationPathArgs;
use crate::extractors::registration_path_args_with_uaid::RegistrationPathArgsWithUaid;
use crate::extractors::router_data_input::RouterDataInput;
use crate::extractors::routers::Routers;
use crate::headers::util::get_header;
use crate::server::ServerState;
use actix_web::web::{Data, Json};
use actix_web::{HttpRequest, HttpResponse};
use autopush_common::db::DynamoDbUser;
use autopush_common::endpoint::make_endpoint;
use cadence::{CountedExt, StatsdClient};
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
        state.settings.endpoint_url().as_str(),
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

/// Handle the `DELETE /v1/{router_type}/{app_id}/registration/{uaid}` route
pub async fn unregister_user_route(
    _auth: AuthorizationCheck,
    path_args: RegistrationPathArgsWithUaid,
    state: Data<ServerState>,
) -> ApiResult<HttpResponse> {
    debug!("Unregistering UAID {}", path_args.uaid);
    state.ddb.remove_user(path_args.uaid).await?;
    Ok(HttpResponse::Ok().finish())
}

/// Handle the `PUT /v1/{router_type}/{app_id}/registration/{uaid}` route
pub async fn update_token_route(
    _auth: AuthorizationCheck,
    path_args: RegistrationPathArgsWithUaid,
    router_data_input: RouterDataInput,
    routers: Routers,
    state: Data<ServerState>,
) -> ApiResult<HttpResponse> {
    // Re-register with router
    debug!(
        "Updating the token of UAID {} with the {} router",
        path_args.uaid, path_args.router_type
    );
    trace!("token = {}", router_data_input.token);
    let router = routers.get(path_args.router_type);
    let router_data = router.register(&router_data_input, &path_args.app_id)?;

    // Update the user in the database
    let user = DynamoDbUser {
        uaid: path_args.uaid,
        router_type: path_args.router_type.to_string(),
        router_data: Some(router_data),
        ..Default::default()
    };
    trace!("Updating user with UAID {}", user.uaid);
    trace!("user = {:?}", user);
    state.ddb.update_user(&user).await?;

    trace!("Finished updating token for UAID {}", user.uaid);
    Ok(HttpResponse::Ok().finish())
}

/// Handle the `POST /v1/{router_type}/{app_id}/registration/{uaid}/subscription` route
pub async fn new_channel_route(
    _auth: AuthorizationCheck,
    path_args: RegistrationPathArgsWithUaid,
    channel_data: Option<Json<NewChannelData>>,
    state: Data<ServerState>,
) -> ApiResult<HttpResponse> {
    // Add the channel
    debug!("Adding a channel to UAID {}", path_args.uaid);
    let channel_data = channel_data.map(Json::into_inner).unwrap_or_default();
    let channel_id = channel_data.channel_id.unwrap_or_else(Uuid::new_v4);
    trace!("channel_id = {}", channel_id);
    state.ddb.add_channel(path_args.uaid, channel_id).await?;

    // Make the endpoint URL
    trace!("Creating endpoint for the new channel");
    let endpoint_url = make_endpoint(
        &path_args.uaid,
        &channel_id,
        channel_data.key.as_deref(),
        state.settings.endpoint_url().as_str(),
        &state.fernet,
    )
    .map_err(ApiErrorKind::EndpointUrl)?;
    trace!("endpoint = {}", endpoint_url);

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "channelID": channel_id,
        "endpoint": endpoint_url,
    })))
}

/// Handle the `GET /v1/{router_type}/{app_id}/registration/{uaid}` route
pub async fn get_channels_route(
    _auth: AuthorizationCheck,
    path_args: RegistrationPathArgsWithUaid,
    state: Data<ServerState>,
) -> ApiResult<HttpResponse> {
    debug!("Getting channel IDs for UAID {}", path_args.uaid);
    let channel_ids = state.ddb.get_channels(path_args.uaid).await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "uaid": path_args.uaid,
        "channelIDs": channel_ids
    })))
}

/// Handle the `DELETE /v1/{router_type}/{app_id}/registration/{uaid}/subscription/{chid}` route
pub async fn unregister_channel_route(
    _auth: AuthorizationCheck,
    path_args: RegistrationPathArgsWithUaid,
    state: Data<ServerState>,
    request: HttpRequest,
) -> ApiResult<HttpResponse> {
    let channel_id = request
        .match_info()
        .get("chid")
        .expect("{chid} must be part of the path")
        .parse::<Uuid>()
        .map_err(|_| ApiErrorKind::NoSubscription)?;

    debug!(
        "Unregistering CHID {} for UAID {}",
        channel_id, path_args.uaid
    );

    incr_metric("ua.command.unregister", &state.metrics, &request);
    let channel_did_exist = state.ddb.remove_channel(path_args.uaid, channel_id).await?;

    if channel_did_exist {
        Ok(HttpResponse::Ok().finish())
    } else {
        debug!("Channel did not exist");
        Err(ApiErrorKind::NoSubscription.into())
    }
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
