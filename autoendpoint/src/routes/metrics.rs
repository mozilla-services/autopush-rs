use actix_web::{HttpResponse, web::Data};
use autopush_common::delivery_metrics::CONTENT_TYPE;

use crate::{
    error::{ApiErrorKind, ApiResult},
    server::AppState,
};

/// Handle the `/metrics` request by returning a Prometheus compatible report.
pub async fn metrics_route(app_state: Data<AppState>) -> ApiResult<HttpResponse> {
    #[cfg(feature = "reliable_report")]
    update_reliability_report(&app_state).await;

    let report = app_state
        .delivery_metrics
        .encode()
        .map_err(|e| ApiErrorKind::General(format!("Metrics report error: {e}")))?;
    Ok(HttpResponse::Ok().content_type(CONTENT_TYPE).body(report))
}

/// Refresh the legacy reliability state gauges. Failures are logged rather than
/// returned so the delivery histograms are still served if Redis is unavailable.
#[cfg(feature = "reliable_report")]
async fn update_reliability_report(app_state: &AppState) {
    let reliability = &app_state.reliability;
    if let Err(e) = reliability.gc().await {
        warn!("🔍⚠️ Reliability gc error: {e}");
    }
    let Some(family) = &app_state.delivery_metrics.reliability_report else {
        return;
    };
    match reliability.report().await {
        Ok(values) => autopush_common::reliability::update_report(family, values),
        Err(e) => {
            // Serve no reliability gauges rather than the last known values
            family.clear();
            warn!("🔍⚠️ Reliability report error: {e}");
        }
    }
}
