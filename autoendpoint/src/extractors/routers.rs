use crate::error::ApiError;
use crate::routers::adm::router::AdmRouter;
use crate::routers::apns::router::ApnsRouter;
use crate::routers::fcm::router::FcmRouter;
use crate::routers::webpush::WebPushRouter;
use crate::routers::Router;
use crate::server::ServerState;
use actix_http::Payload;
use actix_web::web::Data;
use actix_web::{FromRequest, HttpRequest};
use futures::future;
use std::fmt::{self, Display};
use std::str::FromStr;
use std::sync::Arc;

/// Valid `DynamoDbUser::router_type` values
#[derive(Copy, Clone, Debug, PartialEq)]
#[allow(clippy::upper_case_acronyms)]
pub enum RouterType {
    WebPush,
    FCM,
    GCM,
    APNS,
    ADM,
}

impl FromStr for RouterType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "webpush" => Ok(RouterType::WebPush),
            "fcm" => Ok(RouterType::FCM),
            "gcm" => Ok(RouterType::GCM),
            "apns" => Ok(RouterType::APNS),
            "adm" => Ok(RouterType::ADM),
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
            RouterType::ADM => "adm",
        })
    }
}

/// Holds the various notification routers. The routers use resources from the
/// server state, which is why `Routers` is an extractor.
pub struct Routers {
    webpush: WebPushRouter,
    fcm: Arc<FcmRouter>,
    apns: Arc<ApnsRouter>,
    adm: Arc<AdmRouter>,
}

impl FromRequest for Routers {
    type Error = ApiError;
    type Future = future::Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        let state = Data::<ServerState>::extract(req)
            .into_inner()
            .expect("No server state found");

        future::ok(Routers {
            webpush: WebPushRouter {
                db_client: state.db_client.clone(),
                metrics: state.metrics.clone(),
                http: state.http.clone(),
                endpoint_url: state.settings.endpoint_url(),
            },
            fcm: state.fcm_router.clone(),
            apns: state.apns_router.clone(),
            adm: state.adm_router.clone(),
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
            RouterType::ADM => self.adm.as_ref(),
        }
    }
}
