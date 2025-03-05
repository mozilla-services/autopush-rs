use actix_web::{web::Data, HttpResponse};

use crate::{
    error::{ApiErrorKind, ApiResult},
    server::AppState,
};

pub async fn report_handler(app_state: Data<AppState>) -> ApiResult<HttpResponse> {
    let reliability = &app_state.reliability;

    autopush_common::reliability::report_handler(reliability)
        .await
        .map_err(|e| ApiErrorKind::General(format!("Reliability report error: {e}")).into())
}
