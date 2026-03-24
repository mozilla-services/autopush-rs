//! Health and Dockerflow routes
use std::collections::HashMap;
use std::fs::read_to_string;
use std::thread;

use actix_web::{
    web::{Data, Json},
    HttpResponse,
};
use reqwest::StatusCode;
use serde_json::json;

use crate::error::{ApiErrorKind, ApiResult};
use crate::server::AppState;
use autopush_common::db::error::DbResult;
#[cfg(feature = "reliable_report")]
use autopush_common::errors::ApcError;
#[cfg(feature = "reliable_report")]
use autopush_common::metric_name::MetricName;
#[cfg(feature = "reliable_report")]
use autopush_common::metrics::StatsdClientExt;
#[cfg(feature = "reliable_report")]
use autopush_common::util::b64_encode_url;

/// get the local memory usage in bytes (presumes running under kubernetes)
pub async fn memory_usage_percentage(memory_path: &str) -> Option<f64> {
    // If we can read (and there is a limit)
    if let Ok(mem_limit_str) = read_to_string(format!("{}/{}", memory_path, "memory.max")) {
        if mem_limit_str.trim() != "max" {
            if let Ok(mem_limit) = mem_limit_str.trim().parse::<u64>() {
                // get the current memory usage snapshot
                if let Ok(mem_current_str) =
                    read_to_string(format!("{}/{}", memory_path, "memory.current"))
                {
                    if let Ok(mem_current) = mem_current_str.trim().parse::<u64>() {
                        // Stars have aligned, and we can return a value.
                        return Some((mem_current as f64 / mem_limit as f64) * 100.0);
                    }
                }
            }
        }
    }

    None
}

/// Handle the `/health` and `/__heartbeat__` routes
pub async fn health_route(state: Data<AppState>) -> Json<serde_json::Value> {
    let router_health = interpret_table_health(state.db.router_table_exists().await);
    let message_health = interpret_table_health(state.db.message_table_exists().await);
    let mut routers: HashMap<&str, bool> = HashMap::new();
    routers.insert("apns", state.apns_router.active());
    routers.insert("fcm", state.fcm_router.active());

    let mut health = json!({
        "status": if state
            .db
            .health_check()
            .await
            .map_err(|e| {
                error!("Autoendpoint health error: {:?}", e);
                e
            })
            .is_ok() {
            "OK"
        } else {
            "ERROR"
        },
        "version": env!("CARGO_PKG_VERSION"),
        "router_table": router_health,
        "message_table": message_health,
        "routers": routers,
        "request_count":state.in_process_subscription_updates.load(std::sync::atomic::Ordering::Relaxed),
    });

    // if we can display memory usage, do so.
    if let Some(mem_usage) = memory_usage_percentage(&state.settings.kubernetes_memory_path).await {
        health["memory_usage_percentage"] = json!(mem_usage);
    }

    #[cfg(feature = "reliable_report")]
    {
        let reliability_health: Result<String, ApcError> = state
            .reliability
            .health_check()
            .await
            .map(|_| {
                let keys: Vec<String> = state
                    .settings
                    .tracking_keys()
                    .unwrap_or_default()
                    .iter()
                    .map(|k|
                        // Hint the key values
                        b64_encode_url(k)[..8].to_string())
                    .collect();
                if keys.is_empty() {
                    Ok("NO_TRACKING_KEYS".to_owned())
                } else {
                    Ok(format!("OK: {}", keys.join(",")))
                }
            })
            .unwrap_or_else(|e| {
                // Record that Redis is down.
                state
                    .metrics
                    .incr_with_tags(MetricName::ReliabilityErrorRedisUnavailable)
                    .with_tag("application", "autoendpoint")
                    .send();
                error!("🔍🟥 Reliability reporting down: {:?}", e);
                Ok("STORE_ERROR".to_owned())
            });
        health["reliability"] = json!(reliability_health);
    }
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
