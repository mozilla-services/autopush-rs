use crate::error::{ApiError, ApiErrorKind};
use crate::extractors::{registration_path_args::RegistrationPathArgs, routers::RouterType};
use actix_web::{dev::Payload, web, FromRequest, HttpRequest};
use futures::{future::LocalBoxFuture, FutureExt};
use lazy_static::lazy_static;
use regex::Regex;
use uuid::Uuid;

lazy_static! {
    static ref VALID_TOKEN: Regex = Regex::new(r"^[^ ]{8,}$").unwrap();
    static ref VALID_ADM_TOKEN: Regex =
        Regex::new(r"^amzn1.adm-registration.v3.[^ ]{256,}$").unwrap();
}

/// Extracts the router data from the request body and validates the token
/// against the given router's token schema (taken from request path params).
#[derive(serde::Deserialize)]
pub struct RouterDataInput {
    pub token: String,
    #[serde(rename = "channelID")]
    pub channel_id: Option<Uuid>,
    pub key: Option<String>,
    pub aps: Option<String>,
}

impl FromRequest for RouterDataInput {
    type Error = ApiError;
    type Future = LocalBoxFuture<'static, Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        let req = req.clone();
        let mut payload = payload.take();

        async move {
            let path_args = RegistrationPathArgs::extract(&req).into_inner()?;
            let data: web::Json<Self> = web::Json::from_request(&req, &mut payload)
                .await
                .map_err(ApiErrorKind::PayloadError)?;

            // Validate the token according to each router's token schema
            let is_valid = match path_args.router_type {
                RouterType::WebPush => true,
                RouterType::FCM | RouterType::GCM | RouterType::APNS => {
                    VALID_TOKEN.is_match(&data.token)
                }
                RouterType::ADM => VALID_ADM_TOKEN.is_match(&data.token),
                RouterType::STUB => data.token.as_str() == "success",
            };

            if !is_valid {
                return Err(ApiErrorKind::InvalidRouterToken.into());
            }

            Ok(data.into_inner())
        }
        .boxed_local()
    }
}
