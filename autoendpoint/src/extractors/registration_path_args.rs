use crate::error::{ApiError, ApiErrorKind};
use crate::extractors::routers::RouterType;
use crate::server::AppState;
use actix_web::{dev::Payload, web::Data, FromRequest, HttpRequest};
use futures::future;

/// Extracts and validates the `router_type` and `app_id` path arguments
pub struct RegistrationPathArgs {
    pub router_type: RouterType,
    pub app_id: String,
}

impl FromRequest for RegistrationPathArgs {
    type Error = ApiError;
    type Future = future::Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        let match_info = req.match_info();
        let router_type = match match_info
            .get("router_type")
            .expect("{router_type} must be part of the path")
            .parse::<RouterType>()
        {
            Ok(RouterType::STUB) if !stub_router_enabled(req) => {
                return future::err(ApiErrorKind::InvalidRouterType.into())
            }
            Ok(router_type) => router_type,
            Err(_) => return future::err(ApiErrorKind::InvalidRouterType.into()),
        };
        let app_id = match_info
            .get("app_id")
            .expect("{app_id} must be part of the path")
            .to_string();

        future::ok(Self {
            router_type,
            app_id,
        })
    }
}

fn stub_router_enabled(req: &HttpRequest) -> bool {
    req.app_data::<Data<AppState>>()
        .expect("Couldn't get AppState")
        .settings
        .stub
        .enabled
}
