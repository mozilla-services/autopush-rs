use actix_http::ws::HandshakeError;
use actix_web::{error::ResponseError, http::StatusCode, HttpResponse};
use backtrace::Backtrace;
use serde_json::json;

use autopush_common::{db::bigtable::BigTableError, errors::ReportableError};

/// The main error type
#[derive(thiserror::Error, Debug)]
pub enum ApiError {
    #[error("Actix Web error: {0}")]
    Actix(#[from] actix_web::error::Error),

    #[error("LogCheck")]
    LogCheck,
}

impl ResponseError for ApiError {
    fn status_code(&self) -> StatusCode {
        match self {
            ApiError::Actix(e) => e.as_response_error().status_code(),
            ApiError::LogCheck => StatusCode::IM_A_TEAPOT,
        }
    }

    fn error_response(&self) -> HttpResponse {
        let code = self.status_code();
        HttpResponse::build(code).json(json!({
            "code": code.as_u16(),
            "errno": self.errno(),
            "error": self.to_string(),
        }))
    }
}

impl ReportableError for ApiError {
    fn backtrace(&self) -> Option<&Backtrace> {
        None
    }

    fn is_sentry_event(&self) -> bool {
        match self {
            // Ignore failing upgrade to WebSocket
            ApiError::Actix(e) => e.as_error() != Some(&HandshakeError::NoWebsocketUpgrade),
            _ => true,
        }
    }

    fn metric_label(&self) -> Option<&'static str> {
        None
    }
}

impl ApiError {
    /// Return a unique errno code per variant
    pub fn errno(&self) -> i32 {
        match self {
            ApiError::Actix(_) => 500,
            ApiError::LogCheck => 999,
        }
    }
}
