//! Health and Dockerflow routes
use std::thread;

use actix_web::web::{Data, Json};
use actix_web::HttpResponse;
use reqwest::StatusCode;
use serde_json::json;

use autoconnect_settings::AppState;

/// Handle the `/health` and `/__heartbeat__` routes
pub async fn health_route(_state: Data<AppState>) -> Json<serde_json::Value> {
    //TODO: query local state and report results
    Json(json!({
        "status": "OK",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// Handle the `/status` route
pub async fn status_route() -> Json<serde_json::Value> {
    Json(json!({
        "status": "OK",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// Handle the `/__lbheartbeat__` route
pub async fn lb_heartbeat_route() -> HttpResponse {
    // Used by the load balancers, just return OK.
    HttpResponse::Ok().finish()
}

/// Handle the `/__version__` route
pub async fn version_route() -> HttpResponse {
    // Return the contents of the version.json file created by circleci
    // and stored in the docker root
    HttpResponse::Ok()
        .content_type("application/json")
        .body(include_str!("../../../version.json"))
}

/// Handle the `/v1/err` route
pub async fn log_check() -> HttpResponse {
    error!(
        "Test Critical Message";
        "status_code" => StatusCode::IM_A_TEAPOT.as_u16(),
        "errno" => 999,
    );

    thread::spawn(|| {
        panic!("LogCheck");
    });

    HttpResponse::new(StatusCode::IM_A_TEAPOT)
}
