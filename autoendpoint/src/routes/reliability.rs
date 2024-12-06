use actix_web::{web::Data, HttpResponse};
use serde_json::json;

use crate::server::AppState;

pub async fn report_handler(app_state: Data<AppState>) -> HttpResponse {
    let reliability = app_state.reliability.clone();
    match reliability.report().await {
        Ok(Some(v)) => {
            debug!("🔍 Reporting {:?}", &v);
            HttpResponse::Ok()
                .content_type("application/json")
                .body(json!(v).to_string())
        }
        Ok(None) => {
            debug!("🔍 Reporting, but nothing to report");
            HttpResponse::Ok()
                .content_type("application/json")
                .body(json!({"error": "No data"}).to_string())
        }
        Err(e) => {
            debug!("🔍🟥 Reporting, Error {:?}", &e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .body(json!({"error": e.to_string()}).to_string())
        }
    }
}
