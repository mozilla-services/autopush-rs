use crate::server::extractors::routers::Routers;
use actix_web::HttpResponse;

/// Handle the `POST /v1/{router_type}/{app_id}/registration` route
pub async fn register_uaid_route(routers: Routers) -> HttpResponse {
    HttpResponse::Ok().finish()
}
