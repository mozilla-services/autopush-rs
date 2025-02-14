use actix_web::{web::Data, HttpResponse};

use crate::server::AppState;

pub async fn report_handler(app_state: Data<AppState>) -> HttpResponse {
    let reliability = &app_state.reliability;

    autopush_common::reliability::report_handler(reliability).await
}
