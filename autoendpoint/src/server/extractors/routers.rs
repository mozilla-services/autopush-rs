use crate::server::routers::webpush::WebPushRouter;
use crate::server::routers::Router;
use crate::server::ServerState;
use actix_web::dev::{Payload, PayloadStream};
use actix_web::web::Data;
use actix_web::{FromRequest, HttpRequest};
use futures::future;
use std::str::FromStr;

/// Valid `DynamoDbUser::router_type` values
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum RouterType {
    WebPush,
    GCM,
    FCM,
    APNS,
    ADM,
}

impl FromStr for RouterType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "webpush" => Ok(RouterType::WebPush),
            "gcm" => Ok(RouterType::GCM),
            "fcm" => Ok(RouterType::FCM),
            "apns" => Ok(RouterType::APNS),
            "adm" => Ok(RouterType::ADM),
            _ => Err(()),
        }
    }
}

/// Holds the various notification routers. The routers use resources from the
/// server state, which is why `Routers` is an extractor.
pub struct Routers {
    pub webpush: WebPushRouter,
}

impl FromRequest for Routers {
    type Error = ();
    type Future = future::Ready<Result<Self, ()>>;
    type Config = ();

    fn from_request(req: &HttpRequest, _: &mut Payload<PayloadStream>) -> Self::Future {
        let state = Data::<ServerState>::extract(&req)
            .into_inner()
            .expect("No server state found");

        future::ok(Routers {
            webpush: WebPushRouter {
                ddb: state.ddb.clone(),
                metrics: state.metrics.clone(),
                http: state.http.clone(),
                endpoint_url: state.settings.endpoint_url.clone(),
            },
        })
    }
}

impl Routers {
    /// Get the router which handles the router type
    pub fn get(&self, router_type: RouterType) -> &dyn Router {
        match router_type {
            RouterType::WebPush => &self.webpush,
            RouterType::GCM => unimplemented!(),
            RouterType::FCM => unimplemented!(),
            RouterType::APNS => unimplemented!(),
            RouterType::ADM => unimplemented!(),
        }
    }
}
