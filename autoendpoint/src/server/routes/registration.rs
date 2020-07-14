use crate::server::extractors::registration_path_args::RegistrationPathArgs;
use crate::server::extractors::router_token::RouterToken;
use crate::server::extractors::routers::Routers;
use actix_web::HttpResponse;

/// Handle the `POST /v1/{router_type}/{app_id}/registration` route
pub async fn register_uaid_route(
    path_args: RegistrationPathArgs,
    router_token: RouterToken,
    routers: Routers,
) -> HttpResponse {
    HttpResponse::Ok().finish()
}
