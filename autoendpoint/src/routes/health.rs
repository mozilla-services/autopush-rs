//! Health and Dockerflow routes
use std::collections::HashMap;
use std::thread;

use actix_web::{
    web::{Data, Json},
    HttpResponse,
};
use reqwest::StatusCode;
use serde_json::json;

use autopush_common::db::error::DbResult;

use crate::error::{ApiErrorKind, ApiResult};
use crate::server::AppState;

/// Handle the `/health` and `/__heartbeat__` routes
pub async fn health_route(state: Data<AppState>) -> Json<serde_json::Value> {
    let router_health = interpret_table_health(state.db.router_table_exists().await);
    let message_health = interpret_table_health(state.db.message_table_exists().await);
    let mut routers: HashMap<&str, bool> = HashMap::new();
    #[cfg(feature = "adm")]
    routers.insert("adm", state.adm_router.active());
    routers.insert("apns", state.apns_router.active());
    routers.insert("fcm", state.fcm_router.active());

    let health = json!({
    "status": "OK",
    "version": env!("CARGO_PKG_VERSION"),
    "router_table": router_health,
    "message_table": message_health,
    "routers": routers});

    Json(health)
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
        Err(e) => {
            error!("Autoendpoint health error: {:?}", e);
            json!({
                "status": "NOT OK",
                "cause": e.to_string()
            })
        }
    }
}

/// Handle the `/status` route
pub async fn status_route() -> ApiResult<Json<serde_json::Value>> {
    Ok(Json(json!({
        "status": "OK",
        "version": env!("CARGO_PKG_VERSION"),
    })))
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
        "errno" => 999,
    );

    thread::spawn(|| {
        panic!("LogCheck");
    });

    Err(ApiErrorKind::LogCheck.into())
}
