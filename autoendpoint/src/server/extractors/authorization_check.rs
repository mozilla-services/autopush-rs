use crate::auth::sign_with_key;
use crate::error::{ApiError, ApiErrorKind};
use crate::server::headers::util::get_header;
use crate::server::ServerState;
use actix_web::dev::{Payload, PayloadStream};
use actix_web::web::Data;
use actix_web::{FromRequest, HttpRequest};
use futures::future::LocalBoxFuture;
use futures::FutureExt;
use uuid::Uuid;

/// Verifies the request authorization via the authorization header.
///
/// The expected token is the HMAC-SHA256 hash of the UAID, signed with one of
/// the available keys (allows for key rotation).
pub struct AuthorizationCheck;

impl FromRequest for AuthorizationCheck {
    type Error = ApiError;
    type Future = LocalBoxFuture<'static, Result<Self, Self::Error>>;
    type Config = ();

    fn from_request(req: &HttpRequest, _: &mut Payload<PayloadStream>) -> Self::Future {
        let req = req.clone();

        async move {
            let uaid = req
                .match_info()
                .get("uaid")
                .expect("{uaid} must be part of the path")
                .parse::<Uuid>()
                .map_err(|_| ApiErrorKind::NoUser)?;
            let state: Data<ServerState> = Data::extract(&req)
                .into_inner()
                .expect("No server state found");
            let auth_header =
                get_header(&req, "Authorization").ok_or(ApiErrorKind::InvalidAuthentication)?;
            let token = get_token_from_auth_header(auth_header)
                .ok_or(ApiErrorKind::InvalidAuthentication)?;

            // Check the token against the expected token for each key
            for key in state.settings.auth_keys() {
                let expected_token = sign_with_key(key.as_bytes(), uaid.as_bytes())
                    .map_err(ApiErrorKind::RegistrationSecretHash)?;

                if expected_token.len() == token.len()
                    && openssl::memcmp::eq(expected_token.as_bytes(), token.as_bytes())
                {
                    return Ok(Self);
                }
            }

            Err(ApiErrorKind::InvalidAuthentication.into())
        }
        .boxed_local()
    }
}

/// Get the token from a bearer authorization header
fn get_token_from_auth_header(header: &str) -> Option<&str> {
    let mut split = header.splitn(2, ' ');
    let scheme = split.next()?;

    if scheme.to_lowercase() != "bearer" {
        return None;
    }

    split.next()
}
