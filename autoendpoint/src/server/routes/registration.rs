use crate::error::ApiResult;
use crate::server::extractors::registration_path_args::RegistrationPathArgs;
use crate::server::extractors::router_token::RouterToken;
use crate::server::extractors::routers::Routers;
use crate::server::headers::util::get_header;
use crate::server::ServerState;
use actix_web::web::Data;
use actix_web::{HttpRequest, HttpResponse};
use autopush_common::db::DynamoDbUser;
use cadence::Counted;

/// Handle the `POST /v1/{router_type}/{app_id}/registration` route
pub async fn register_uaid_route(
    path_args: RegistrationPathArgs,
    router_token: RouterToken,
    routers: Routers,
    state: Data<ServerState>,
    request: HttpRequest,
) -> ApiResult<HttpResponse> {
    // Register with router
    let router = routers.get(path_args.router_type);
    let router_data = router.register(&router_token.token, &path_args.app_id)?;

    state
        .metrics
        .incr_with_tags("ua.command.register")
        .with_tag(
            "user_agent",
            get_header(&request, "User-Agent").unwrap_or("unknown"),
        )
        .with_tag("host", get_header(&request, "Host").unwrap_or("unknown"))
        .send();

    // Register user and channel in database
    let user = DynamoDbUser {
        router_type: path_args.router_type.to_string(),
        router_data: Some(router_data),
        ..Default::default()
    };
    state.ddb.add_user(&user).await?;

    // TODO: Generate and return endpoint

    Ok(HttpResponse::Ok().finish())
}
