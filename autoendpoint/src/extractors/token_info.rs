use crate::error::{ApiError, ApiErrorKind};
use crate::headers::util::get_owned_header;
use actix_http::{Payload, PayloadStream};
use actix_web::{FromRequest, HttpRequest};
use futures::future;
use std::str::FromStr;

/// Extracts basic token data from the webpush request path and headers
pub struct TokenInfo {
    pub api_version: ApiVersion,
    pub token: String,
    pub crypto_key_header: Option<String>,
    pub auth_header: Option<String>,
}

impl FromRequest for TokenInfo {
    type Error = ApiError;
    type Future = future::Ready<Result<Self, Self::Error>>;
    type Config = ();

    fn from_request(req: &HttpRequest, _: &mut Payload<PayloadStream>) -> Self::Future {
        // Path variables
        let api_version = match req.match_info().get("api_version").unwrap_or("v1").parse() {
            Ok(version) => version,
            Err(e) => return future::err(e),
        };
        let token = req
            .match_info()
            .get("token")
            .expect("{token} must be part of the webpush path")
            .to_string();

        future::ok(TokenInfo {
            api_version,
            token,
            crypto_key_header: get_owned_header(req, "crypto-key"),
            auth_header: get_owned_header(req, "authorization"),
        })
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum ApiVersion {
    Version1,
    Version2,
}

impl FromStr for ApiVersion {
    type Err = ApiError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "v1" => Ok(ApiVersion::Version1),
            "v2" => Ok(ApiVersion::Version2),
            _ => Err(ApiErrorKind::InvalidApiVersion.into()),
        }
    }
}
