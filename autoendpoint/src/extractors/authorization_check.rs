use crate::auth::sign_with_key;
use crate::error::{ApiError, ApiErrorKind};
use crate::headers::util::get_header;
use crate::server::AppState;
use actix_web::dev::Payload;
use actix_web::{web::Data, FromRequest, HttpRequest};
use futures::future::LocalBoxFuture;
use futures::FutureExt;
use openssl::error::ErrorStack;
use uuid::Uuid;

/// Verifies the request authorization via the authorization header.
///
/// The expected token is the HMAC-SHA256 hash of the UAID, signed with one of
/// the available keys (allows for key rotation).
/// NOTE: This is *ONLY* for internal calls that require authorization and should
///      NOT be used by calls that are using VAPID authentication (e.g.
///      subscription provider endpoints)
pub struct AuthorizationCheck;

impl AuthorizationCheck {
    pub fn generate_token(auth_key: &str, user: &Uuid) -> Result<String, ErrorStack> {
        sign_with_key(auth_key.as_bytes(), user.as_simple().to_string().as_bytes())
    }

    pub fn validate_token(
        token: &str,
        uaid: &Uuid,
        auth_keys: &[String],
    ) -> Result<Self, ApiError> {
        // Check the token against the expected token for each key
        for key in auth_keys {
            let expected_token =
                sign_with_key(key.as_bytes(), uaid.as_simple().to_string().as_bytes())
                    .map_err(ApiErrorKind::RegistrationSecretHash)?;

            debug!("expected: {:?}, recv'd {:?}", &expected_token, &token);
            if expected_token.len() == token.len()
                && openssl::memcmp::eq(expected_token.as_bytes(), token.as_bytes())
            {
                return Ok(Self);
            }
        }
        Err(ApiErrorKind::InvalidLocalAuth("incorrect auth token".to_owned()).into())
    }
}

impl FromRequest for AuthorizationCheck {
    type Error = ApiError;
    type Future = LocalBoxFuture<'static, Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        let req = req.clone();

        async move {
            let uaid = req
                .match_info()
                .get("uaid")
                .expect("{uaid} must be part of the path")
                .parse::<Uuid>()
                .map_err(|_| ApiErrorKind::NoUser)?;
            let state: Data<AppState> = Data::extract(&req)
                .into_inner()
                .expect("No server state found");
            let auth_header = get_header(&req, "Authorization")
                .ok_or_else(|| ApiErrorKind::InvalidLocalAuth("missing auth header".to_owned()))?;
            let token = get_token_from_auth_header(auth_header)
                .ok_or_else(|| ApiErrorKind::InvalidLocalAuth("missing auth token".to_owned()))?;

            Self::validate_token(token, &uaid, &state.settings.auth_keys())
        }
        .boxed_local()
    }
}

/// Get the token from a bearer authorization header
fn get_token_from_auth_header(header: &str) -> Option<&str> {
    let mut split = header.splitn(2, ' ');
    let scheme = split.next()?;

    // An error in the android push component code uses "webpush" to identify
    // the local authorization header. We need to allow for that.
    if !["bearer", "webpush"].contains(&scheme.to_lowercase().as_str()) {
        return None;
    }

    split.next()
}

#[cfg(test)]
mod test {

    use crate::error::ApiResult;

    use super::*;

    #[test]
    fn test_signature() -> ApiResult<()> {
        // hopefully no-op type test to check locally generated tokens.
        let uaid: Uuid = "729e5104f5f04abc9196085340317dea".parse().unwrap();
        let auth_keys = ["HJVPy4ZwF4Yz_JdvXTL8hRcwIhv742vC60Tg5Ycrvw8=".to_owned()].to_vec();
        let token = AuthorizationCheck::generate_token(auth_keys.get(0).unwrap(), &uaid).unwrap();

        AuthorizationCheck::validate_token(&token, &uaid, &auth_keys)?;
        Ok(())
    }

    #[test]
    fn test_legacy_signature() -> ApiResult<()> {
        // check a previously generated python token.
        // original python uaids are lower case all hex.
        let uaid: Uuid = "729e5104f5f04abc9196085340317dea".parse().unwrap();
        // Auth keys are strings. don't run through base64!
        let auth_keys = ["HJVPy4ZwF4Yz_JdvXTL8hRcwIhv742vC60Tg5Ycrvw8=".to_owned()].to_vec();
        // the following token was generated using the old python application.
        let legacy_token = "f694963453adf5dedcc379bbdd6900d692b6e09f1c91f44169bfcd2f941bf36c";
        // pop the firstkey off of the auth_key list.
        let selected = auth_keys.get(0).unwrap();
        let token = AuthorizationCheck::generate_token(selected, &uaid).unwrap();

        assert_eq!(&token, legacy_token);
        Ok(())
    }

    #[test]
    fn test_token_extractor() -> ApiResult<()> {
        let uaid: Uuid = "729e5104f5f04abc9196085340317dea".parse().unwrap();
        let auth_keys = ["HJVPy4ZwF4Yz_JdvXTL8hRcwIhv742vC60Tg5Ycrvw8=".to_owned()].to_vec();
        let token = AuthorizationCheck::generate_token(auth_keys.get(0).unwrap(), &uaid).unwrap();

        assert!(get_token_from_auth_header(&format!("bearer {}", &token)).is_some());
        assert!(get_token_from_auth_header(&format!("webpush {}", &token)).is_some());
        assert!(get_token_from_auth_header(&format!("random {}", &token)).is_none());
        Ok(())
    }
}
