use actix_web::web::{Data, Json};
use actix_web::{HttpRequest, HttpResponse};
use cadence::{Histogrammed, StatsdClient};
use uuid::Uuid;

use crate::error::{ApiErrorKind, ApiResult};
use crate::extractors::{
    authorization_check::AuthorizationCheck, new_channel_data::NewChannelData,
    registration_path_args::RegistrationPathArgs,
    registration_path_args_with_uaid::RegistrationPathArgsWithUaid,
    router_data_input::RouterDataInput, routers::Routers, user::ReqUaid,
};
use crate::headers::util::get_header;
use crate::server::AppState;

use autopush_common::db::User;
use autopush_common::endpoint::make_endpoint;
use autopush_common::metric_name::MetricName;
use autopush_common::metrics::StatsdClientExt;

/// Handle the `POST /v1/{router_type}/{app_id}/registration` route
pub async fn register_uaid_route(
    path_args: RegistrationPathArgs,
    router_data_input: RouterDataInput,
    routers: Routers,
    app_state: Data<AppState>,
    request: HttpRequest,
) -> ApiResult<HttpResponse> {
    // Register with router
    debug!(
        "Registering a user with the {} router",
        path_args.router_type
    );
    trace!("🌍 token = {}", router_data_input.token);
    let router = routers.get(path_args.router_type);
    let router_data = router.register(&router_data_input, &path_args.app_id)?;
    incr_metric(MetricName::UaCommandRegister, &app_state.metrics, &request);

    // Register user and channel in database
    let user = User::builder()
        .router_type(path_args.router_type.to_string())
        .router_data(router_data)
        .build()
        .map_err(|e| ApiErrorKind::General(format!("User::builder error: {e}")))?;
    let channel_id = router_data_input.channel_id.unwrap_or_else(Uuid::new_v4);
    trace!("🌍 Creating user with UAID {}", user.uaid);
    trace!("🌍 user = {:?}", user);
    trace!("🌍 channel_id = {}", channel_id);
    app_state.db.add_user(&user).await?;
    app_state.db.add_channel(&user.uaid, &channel_id).await?;

    // Make the endpoint URL
    trace!("🌍 Creating endpoint for user");
    let endpoint_url = make_endpoint(
        &user.uaid,
        &channel_id,
        router_data_input.key.as_deref(),
        app_state.settings.endpoint_url().as_str(),
        &app_state.fernet,
    )
    .map_err(ApiErrorKind::EndpointUrl)?;
    trace!("🌍 endpoint = {}", endpoint_url);

    // Create the secret
    trace!("🌍 Creating secret for UAID {}", user.uaid);
    let auth_keys = app_state.settings.auth_keys();
    let auth_key = auth_keys
        .first()
        .expect("At least one auth key must be provided in the settings");
    let secret = AuthorizationCheck::generate_token(auth_key, &user.uaid)
        .map_err(ApiErrorKind::RegistrationSecretHash)?;

    trace!("🌍 Finished registering UAID {}", user.uaid);
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
    app_state: Data<AppState>,
) -> ApiResult<HttpResponse> {
    let uaid = path_args.user.uaid;
    debug!("🌍 Unregistering UAID {uaid}");
    app_state.db.remove_user(&uaid).await?;
    Ok(HttpResponse::Ok().finish())
}

/// Handle the `PUT /v1/{router_type}/{app_id}/registration/{uaid}` route
pub async fn update_token_route(
    _auth: AuthorizationCheck,
    path_args: RegistrationPathArgsWithUaid,
    router_data_input: RouterDataInput,
    routers: Routers,
    app_state: Data<AppState>,
) -> ApiResult<HttpResponse> {
    // Re-register with router
    let RegistrationPathArgsWithUaid {
        router_type,
        app_id,
        mut user,
    } = path_args;
    let uaid = user.uaid;
    debug!("🌍 Updating the token of UAID {uaid} with the {router_type} router");
    trace!("token = {}", router_data_input.token);
    let router = routers.get(path_args.router_type);
    let router_data = router.register(&router_data_input, &app_id)?;

    // Update the user in the database
    user.router_type = path_args.router_type.to_string();
    user.router_data = Some(router_data);
    trace!("🌍 Updating user with UAID {uaid}");
    trace!("🌍 user = {user:?}");
    if !app_state.db.update_user(&mut user).await? {
        // Occurs occasionally on mobile records
        return Err(ApiErrorKind::Conditional("update_user".to_owned()).into());
    }

    trace!("🌍 Finished updating token for UAID {uaid}");
    Ok(HttpResponse::Ok().finish())
}

/// Handle the `POST /v1/{router_type}/{app_id}/registration/{uaid}/subscription` route
pub async fn new_channel_route(
    _auth: AuthorizationCheck,
    path_args: RegistrationPathArgsWithUaid,
    channel_data: Option<Json<NewChannelData>>,
    app_state: Data<AppState>,
) -> ApiResult<HttpResponse> {
    // Add the channel
    let uaid = path_args.user.uaid;
    debug!("🌍 Adding a channel to UAID {uaid}");
    let channel_data = channel_data.map(Json::into_inner).unwrap_or_default();
    let channel_id = channel_data.channel_id.unwrap_or_else(Uuid::new_v4);
    trace!("🌍 channel_id = {channel_id}");
    app_state.db.add_channel(&uaid, &channel_id).await?;

    // Make the endpoint URL
    trace!("🌍 Creating endpoint for the new channel");
    let endpoint_url = make_endpoint(
        &uaid,
        &channel_id,
        channel_data.key.as_deref(),
        app_state.settings.endpoint_url().as_str(),
        &app_state.fernet,
    )
    .map_err(ApiErrorKind::EndpointUrl)?;
    trace!("endpoint = {endpoint_url}");

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "channelID": channel_id,
        "endpoint": endpoint_url,
    })))
}

/// Handle the `GET /v1/{router_type}/{app_id}/registration/{uaid}` route
/// Since this is called daily by a mobile device, it can serve as part of
/// a liveliness check for the device. This is more authoritative than
/// relying on the bridge service to return a "success", since the bridge
/// may retain "inactive" devices, but will immediately drop devices
/// where Firefox has been uninstalled or that have been factory reset.
pub async fn get_channels_route(
    auth: AuthorizationCheck,
    path_args: RegistrationPathArgsWithUaid,
    app_state: Data<AppState>,
) -> ApiResult<HttpResponse> {
    let uaid = path_args.user.uaid;
    let db = &app_state.db;
    debug!("🌍 Getting channel IDs for UAID {uaid}");
    //
    if let Some(mut user) = db.get_user(&uaid).await? {
        db.update_user(&mut user).await?;
        // report the rough user agent so we know what clients are actively pinging us.
        let os = auth.user_agent.metrics_os.clone();
        let browser = auth.user_agent.metrics_browser.clone();
        // use the "real" version here. (these are less normalized)
        info!("Mobile client check";
            "os" => auth.user_agent.os,
            "os_version" => auth.user_agent.os_version,
            "browser" => auth.user_agent.browser_name,
            "browser_version" => auth.user_agent.browser_version);
        // use the "metrics" version since we need to consider cardinality.
        app_state
            .metrics
            .incr_with_tags(MetricName::UaConnectionCheck)
            .with_tag("os", &os)
            .with_tag("browser", &browser)
            .send();
    }
    let channel_ids = db.get_channels(&uaid).await?;

    app_state
        .metrics
        .histogram_with_tags(
            MetricName::UaConnectionChannelCount.as_ref(),
            channel_ids.len() as u64,
        )
        .with_tag_value("mobile")
        .send();

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "uaid": uaid,
        "channelIDs": channel_ids
    })))
}

/// Handle the `DELETE /v1/{router_type}/{app_id}/registration/{uaid}/subscription/{chid}` route
pub async fn unregister_channel_route(
    _auth: AuthorizationCheck,
    path_args: RegistrationPathArgsWithUaid,
    app_state: Data<AppState>,
    request: HttpRequest,
) -> ApiResult<HttpResponse> {
    let channel_id = request
        .match_info()
        .get("chid")
        .expect("{chid} must be part of the path")
        .parse::<Uuid>()
        .map_err(|_| ApiErrorKind::NoSubscription)?;
    let uaid = path_args.user.uaid;
    debug!("🌍 Unregistering CHID {channel_id} for UAID {uaid}");

    incr_metric(
        MetricName::UaCommandUnregister,
        &app_state.metrics,
        &request,
    );
    let channel_did_exist = app_state.db.remove_channel(&uaid, &channel_id).await?;

    if channel_did_exist {
        Ok(HttpResponse::Ok().finish())
    } else {
        debug!("Channel did not exist");
        Err(ApiErrorKind::NoSubscription.into())
    }
}

/// Report the status of a given UAID. This was requested by the UA so that
/// it can check to see if a given UAID has been reset. This will mostly be
/// used by desktop clients, since the mobile clients will do a daily check-in.
pub async fn check_uaid(req_uaid: ReqUaid, app_state: Data<AppState>) -> ApiResult<HttpResponse> {
    debug!("🌍 Checking UAID {req_uaid}");
    let mut response = serde_json::json!({
        "uaid": req_uaid.uaid,
    });
    match app_state.db.get_user(&req_uaid.uaid).await {
        Ok(Some(_user)) => {
            debug!("🌍 UAID {req_uaid} good");
            response["status"] = actix_http::StatusCode::OK.as_u16().into();
        }
        Ok(None) => {
            debug!("🌍 UAID {req_uaid} bad");
            response["status"] = actix_http::StatusCode::NOT_FOUND.as_u16().into();
        }
        Err(e) => {
            warn!("🌍⚠ UAID check db bad");
            let err: ApiErrorKind = e.into();
            response["status"] = err.status().as_u16().into();
            response["message"] = err.to_string().into();
            response["errno"] = err.errno().into();
        }
    }
    Ok(HttpResponse::Ok().json(response))
}

/// Increment a metric with data from the request
fn incr_metric(metric: MetricName, metrics: &StatsdClient, request: &HttpRequest) {
    metrics
        .incr_with_tags(metric)
        .with_tag(
            "user_agent",
            get_header(request, "User-Agent").unwrap_or("unknown"),
        )
        .with_tag("host", get_header(request, "Host").unwrap_or("unknown"))
        .send()
}
