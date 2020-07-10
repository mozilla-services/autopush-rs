//! Health and Dockerflow routes

use actix_web::web::Json;
use actix_web::HttpResponse;
use serde_json::json;

/// Handle the `/health` route
pub async fn health_route() -> Json<serde_json::Value> {
    // TODO: Get database table health
    Json(json!({
        "status": "OK"
    }))
}

/// Handle the `/status` and `/__heartbeat__` routes
pub async fn status_route() -> Json<serde_json::Value> {
    Json(json!({
        "status": "OK",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// Handle the `/__lbheartbeat__` route
pub fn lb_heartbeat_route() -> HttpResponse {
    // Used by the load balancers, just return OK.
    HttpResponse::Ok().finish()
}

/// Handle the `/__version__` route
pub fn version_route() -> HttpResponse {
    // Return the contents of the version.json file created by circleci
    // and stored in the docker root
    HttpResponse::Ok()
        .content_type("application/json")
        .body(include_str!("../../../version.json"))
}
