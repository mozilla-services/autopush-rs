use crate::error::{ApiError, ApiResult};
use crate::routers::apns::router::ApnsRouter;
use crate::routers::fcm::router::FcmRouter;
use crate::routers::webpush::WebPushRouter;
use crate::routers::Router;
use crate::server::AppState;
use actix_web::dev::Payload;
use actix_web::web::Data;
use actix_web::{FromRequest, HttpRequest};
use futures::future;
use std::fmt::{self, Display};
use std::str::FromStr;
use std::sync::Arc;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[allow(clippy::upper_case_acronyms)]
pub enum RouterType {
    WebPush,
    FCM,
    GCM,
    APNS,
}

impl FromStr for RouterType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "webpush" => Ok(RouterType::WebPush),
            "fcm" => Ok(RouterType::FCM),
            "gcm" => Ok(RouterType::GCM),
            "apns" => Ok(RouterType::APNS),
            _ => Err(()),
        }
    }
}

impl Display for RouterType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            RouterType::WebPush => "webpush",
            RouterType::FCM => "fcm",
            RouterType::GCM => "gcm",
            RouterType::APNS => "apns",
        })
    }
}

/// Holds the various notification routers. The routers use resources from the
/// server state, which is why `Routers` is an extractor.
pub struct Routers {
    webpush: WebPushRouter,
    fcm: Arc<FcmRouter>,
    apns: Arc<ApnsRouter>,
}

impl FromRequest for Routers {
    type Error = ApiError;
    type Future = future::Ready<ApiResult<Self>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        let app_state = Data::<AppState>::extract(req)
            .into_inner()
            .expect("No server state found");

        future::ok(Routers {
            webpush: WebPushRouter {
                db: app_state.db.clone(),
                metrics: app_state.metrics.clone(),
                http: app_state.http.clone(),
                endpoint_url: app_state.settings.endpoint_url(),
            },
            fcm: app_state.fcm_router.clone(),
            apns: app_state.apns_router.clone(),
        })
    }
}

impl Routers {
    /// Get the router which handles the router type
    pub fn get(&self, router_type: RouterType) -> &dyn Router {
        match router_type {
            RouterType::WebPush => &self.webpush,
            RouterType::FCM | RouterType::GCM => self.fcm.as_ref(),
            RouterType::APNS => self.apns.as_ref(),
        }
    }
}
