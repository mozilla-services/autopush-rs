//! Health and Dockerflow routes

use autopush_common::db::error::DbResult;
use autopush_common::errors::{ApiErrorKind, ApiResult};
use crate::server::ServerOptions;
use actix_web::web::{Data, Json};
use actix_web::HttpResponse;
use reqwest::StatusCode;
use serde_json::json;
use std::thread;

/// Handle the `/health` and `/__heartbeat__` routes
pub async fn health_route(state: Data<ServerOptions>) -> Json<serde_json::Value> {
    let router_health = interpret_table_health(state.db_client.router_table_exists().await);
    let message_health = interpret_table_health(state.db_client.message_table_exists().await);

    Json(json!({
        "status": "OK",
        "version": env!("CARGO_PKG_VERSION"),
        "router_table": router_health,
        "message_table": message_health,
        "routers": {
            "adm": state.adm_router.active(),
            "apns": state.apns_router.active(),
            "fcm": state.fcm_router.active(),
        }
    }))
}

/// Convert the result of a DB health check to JSON
fn interpret_table_health(health: DbResult<bool>) -> serde_json::Value {
    match health {
        Ok(true) => json!({
            "status": "OK"
        }),
        Ok(false) => json!({
            "status": "NOT OK",
            "cause": "Nonexistent table"
        }),
        Err(e) => json!({
            "status": "NOT OK",
            "cause": e.to_string()
        }),
    }
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
pub async fn log_check() -> ApiResult<String> {
    error!(
        "Test Critical Message";
        "status_code" => StatusCode::IM_A_TEAPOT.as_u16(),
        "errno" => 999_u16,
    );

    thread::spawn(|| {
        panic!("LogCheck");
    });

    Err(ApiErrorKind::LogCheck.into())
}
