//! Health and Dockerflow routes
use std::thread;

use actix_web::web::{Data, Json};
use actix_web::HttpResponse;
use reqwest::StatusCode;
use serde_json::json;

use autoconnect_settings::AppState;

/// Handle the `/health` and `/__heartbeat__` routes
pub async fn health_route(state: Data<AppState>) -> Json<serde_json::Value> {
    let status = if state.db.health_check().await.is_ok() {
        "OK"
    } else {
        "ERROR"
    };
    //TODO: query local state and report results
    Json(json!({
        "status": status,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// Handle the `/status` route
pub async fn status_route(state: Data<AppState>) -> Json<serde_json::Value> {
    let mut status:std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    status.insert("version",env!("CARGO_PKG_VERSION").to_owned());
    let check = state.db.health_check().await;
    if check.is_ok() {
        status.insert("status", "OK".to_owned());
    } else {
        status.insert("status", "ERROR".to_owned());
    }
    if let Some(err) = check.err().map(|v| v.to_string()) {
        status.insert("error", err);
    };

     Json(json!(status))
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
