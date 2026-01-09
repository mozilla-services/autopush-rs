//! Health and Dockerflow routes
use std::thread;

use actix_web::{
    web::{self, Data, Json},
    HttpResponse, ResponseError,
};
use serde_json::json;

use autoconnect_settings::AppState;
// The following two items are used by "reliable_report"
// Feature flagging them causes a cascade of other unused imports for some configurations.
#[allow(unused_imports)]
use autopush_common::metric_name::MetricName;
#[allow(unused_imports)]
use autopush_common::metrics::StatsdClientExt;

use crate::error::ApiError;

/// Configure the Dockerflow (and legacy monitoring) routes
pub fn config(config: &mut web::ServiceConfig) {
    config
        .service(web::resource("/status").route(web::get().to(status_route)))
        .service(web::resource("/health").route(web::get().to(health_route)))
        .service(web::resource("/v1/err/crit").route(web::get().to(log_check)))
        // standardized
        .service(web::resource("/__error__").route(web::get().to(log_check)))
        // Dockerflow
        .service(web::resource("/__heartbeat__").route(web::get().to(health_route)))
        .service(web::resource("/__lbheartbeat__").route(web::get().to(lb_heartbeat_route)))
        .service(web::resource("/__version__").route(web::get().to(version_route)));
}

/// Handle the `/health` and `/__heartbeat__` routes
pub async fn health_route(state: Data<AppState>) -> Json<serde_json::Value> {
    #[allow(unused_mut)]
    let mut health = json!({
        "status": if state
        .db
        .health_check()
        .await
        .map_err(|e| {
            error!("Autoconnect Health Error: {:?}", e);
            e
        })
        .is_ok() { "OK" } else {"ERROR"},
        "version": env!("CARGO_PKG_VERSION"),
        "connections": state.clients.count().await
    });

    #[cfg(feature = "reliable_report")]
    {
        health["reliability"] = json!(state.reliability.health_check().await.unwrap_or_else(|e| {
            state
                .metrics
                .incr_with_tags(MetricName::ErrorRedisUnavailable)
                .with_tag("application", "autoconnect")
                .send();
            error!("🔍🟥 Reliability reporting down: {:?}", e);
            "ERROR"
        }));
    }

    Json(health)
}

/// Handle the `/status` route
pub async fn status_route(state: Data<AppState>) -> Json<serde_json::Value> {
    let mut status: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    status.insert("version", env!("CARGO_PKG_VERSION").to_owned());
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
pub async fn log_check() -> Result<HttpResponse, ApiError> {
    let err = ApiError::LogCheck;
    error!(
        "Test Critical Message";
        "status_code" => err.status_code().as_u16(),
        "errno" => err.errno(),
    );

    thread::spawn(|| {
        panic!("LogCheck");
    });

    Err(err)
}
