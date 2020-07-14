use crate::error::{ApiErrorKind, ApiResult};
use crate::server::extractors::registration_path_args::RegistrationPathArgs;
use crate::server::extractors::router_data_input::RouterDataInput;
use crate::server::extractors::routers::Routers;
use crate::server::headers::util::get_header;
use crate::server::ServerState;
use actix_web::web::Data;
use actix_web::{HttpRequest, HttpResponse};
use autopush_common::db::DynamoDbUser;
use cadence::Counted;
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
    let router = routers.get(path_args.router_type);
    let router_data = router.register(&router_data_input, &path_args.app_id)?;

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
    let channel_id = router_data_input.channel_id.unwrap_or_else(Uuid::new_v4);
    state.ddb.add_user(&user).await?;
    state.ddb.add_channel(user.uaid, channel_id).await?;

    // TODO: Generate and return endpoint

    Ok(HttpResponse::Ok().finish())
}
