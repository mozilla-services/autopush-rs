use std::collections::HashMap;

use actix_web::HttpRequest;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::error::{ApiError, ApiErrorKind, ApiResult};
use crate::extractors::subscription::decode_public_key;
use crate::headers::{crypto_key::CryptoKeyHeader, util::split_key_value};

use autopush_common::util::sec_since_epoch;

pub const ALLOWED_SCHEMES: [&str; 3] = ["bearer", "webpush", "vapid"];
pub const ONE_DAY_IN_SECONDS: u64 = 60 * 60 * 24;

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct VapidClaims {
    pub exp: u64,
    pub aud: String,
    pub sub: String,
    pub meta: Option<String>,
}

impl Default for VapidClaims {
    fn default() -> Self {
        Self {
            exp: sec_since_epoch() + ONE_DAY_IN_SECONDS,
            aud: "No audience".to_owned(),
            sub: "No sub".to_owned(),
            meta: None,
        }
    }
}

impl VapidClaims {
    pub fn from_token(token: &str, public_key: &str) -> ApiResult<Self> {
        let public_key = decode_public_key(public_key)?;
        if public_key.len() < 64 || public_key.len() > 65 {
            trace!("⚠ Potentially invalid public key! {}", public_key.len())
        }
        match jsonwebtoken::decode::<VapidClaims>(
            token,
            &DecodingKey::from_ec_der(&public_key),
            &Validation::new(Algorithm::ES256),
        ) {
            Ok(v) => Ok(v.claims),
            Err(e) => match e.kind() {
                // NOTE: This will fail if `exp` is specified as anything instead of a numeric or if a required field is empty
                jsonwebtoken::errors::ErrorKind::Json(e) => {
                    if e.is_data() {
                        return Err(VapidError::InvalidVapid(
                            "A value in the vapid claims is either missing or incorrectly specified (e.g. \"exp\":\"12345\" or \"sub\":null). Please correct and retry.".to_owned(),
                        )
                        .into());
                    }
                    // Other errors are always possible. Try to be helpful by returning
                    // the Json parse error.
                    Err(VapidError::InvalidVapid(e.to_string()).into())
                }
                _ => Err(e.into()),
            },
        }
    }
}

impl TryFrom<&HttpRequest> for VapidClaims {
    type Error = ApiError;

    fn try_from(req: &HttpRequest) -> Result<Self, Self::Error> {
        let header = req
            .headers()
            .get("Authorization")
            .ok_or(ApiErrorKind::InvalidAuthentication)?;
        let vapid_header = VapidHeader::parse(
            header
                .to_str()
                .map_err(|_| ApiErrorKind::InvalidAuthentication)?,
        )?;
        let key = match vapid_header.version_data {
            VapidVersionData::Version1 => {
                let cheader_raw = req
                    .headers()
                    .get("Crypto-Key")
                    .ok_or(ApiErrorKind::InvalidAuthentication)?
                    .to_str()
                    .map_err(|_| ApiErrorKind::InvalidAuthentication)?;
                let cheader = CryptoKeyHeader::parse(cheader_raw)
                    .ok_or(ApiErrorKind::InvalidAuthentication)?;
                cheader
                    .get_by_key("p256ecdsa")
                    .ok_or(ApiErrorKind::InvalidAuthentication)?
                    .to_owned()
            }
            VapidVersionData::Version2 { public_key } => public_key,
        };
        VapidClaims::from_token(&vapid_header.token, &key)
    }
}
/// Parses the VAPID authorization header
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VapidHeader {
    pub scheme: String,
    pub token: String,
    pub version_data: VapidVersionData,
}

/// Combines the VAPID header details with the public key, which may not be from
/// the VAPID header
#[derive(Clone, Debug)]
pub struct VapidHeaderWithKey {
    pub vapid: VapidHeader,
    pub public_key: String,
}

/// Version-specific VAPID data. Also used to identify the VAPID version.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VapidVersionData {
    Version1,
    Version2 { public_key: String },
}

impl VapidHeader {
    /// Parse the VAPID authorization header. The public key is available if the
    /// version is 2 ("vapid" scheme).
    pub fn parse(header: &str) -> Result<VapidHeader, VapidError> {
        let mut scheme_split = header.splitn(2, ' ');
        let scheme = scheme_split
            .next()
            .ok_or(VapidError::MissingToken)?
            .to_lowercase();
        let data = scheme_split
            .next()
            .ok_or(VapidError::MissingToken)?
            .replace(' ', "");

        if !ALLOWED_SCHEMES.contains(&scheme.as_str()) {
            return Err(VapidError::UnknownScheme);
        }

        let (token, version_data) = if scheme == "vapid" {
            let data: HashMap<&str, &str> = data.split(',').filter_map(split_key_value).collect();

            let public_key = *data.get("k").ok_or(VapidError::MissingKey)?;
            let token = *data.get("t").ok_or(VapidError::MissingToken)?;

            (
                token.to_string(),
                VapidVersionData::Version2 {
                    public_key: public_key.to_string(),
                },
            )
        } else {
            (data, VapidVersionData::Version1)
        };

        Ok(Self {
            scheme,
            token,
            version_data,
        })
    }

    /// Get the VAPID version as a number
    pub fn version(&self) -> usize {
        match self.version_data {
            VapidVersionData::Version1 => 1,
            VapidVersionData::Version2 { .. } => 2,
        }
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum VapidError {
    #[error("Missing VAPID token")]
    MissingToken,
    #[error("Invalid VAPID token: {0}")]
    InvalidVapid(String),
    #[error("Missing VAPID public key")]
    MissingKey,
    #[error("Invalid VAPID public key: {0}")]
    InvalidKey(String),
    #[error("Invalid VAPID audience")]
    InvalidAudience,
    #[error("Invalid VAPID expiry")]
    InvalidExpiry,
    #[error("VAPID public key mismatch")]
    KeyMismatch,
    #[error("The VAPID token expiration is too long")]
    FutureExpirationToken,
    #[error("Unknown auth scheme")]
    UnknownScheme,
}

#[cfg(test)]
mod tests {
    use super::{VapidHeader, VapidVersionData};

    const TOKEN: &str = "eyJ0eXAiOiJKV1QiLCJhbGciOiJFUzI1NiJ9.eyJhdWQiOiJodHRwc\
        zovL3B1c2guc2VydmljZXMubW96aWxsYS5jb20iLCJzdWIiOiJtYWlsdG86YWRtaW5AZXhh\
        bXBsZS5jb20iLCJleHAiOiIxNDYzMDAxMzQwIn0.y_dvPoTLBo60WwtocJmaTWaNet81_jT\
        TJuyYt2CkxykLqop69pirSWLLRy80no9oTL8SDLXgTaYF1OrTIEkDow";
    const KEY: &str = "BAS7pgV_RFQx5yAwSePfrmjvNm1sDXyMpyDSCL1IXRU32cdtopiAmSys\
        WTCrL_aZg2GE1B_D9v7weQVXC3zDmnQ";
    const VALID_HEADER: &str = "vapid t=eyJ0eXAiOiJKV1QiLCJhbGciOiJFUzI1NiJ9.ey\
        JhdWQiOiJodHRwczovL3B1c2guc2VydmljZXMubW96aWxsYS5jb20iLCJzdWIiOiJtYWlsd\
        G86YWRtaW5AZXhhbXBsZS5jb20iLCJleHAiOiIxNDYzMDAxMzQwIn0.y_dvPoTLBo60Wwto\
        cJmaTWaNet81_jTTJuyYt2CkxykLqop69pirSWLLRy80no9oTL8SDLXgTaYF1OrTIEkDow,\
        k=BAS7pgV_RFQx5yAwSePfrmjvNm1sDXyMpyDSCL1IXRU32cdtopiAmSysWTCrL_aZg2GE1\
        B_D9v7weQVXC3zDmnQ";

    #[test]
    fn parse_succeeds() {
        assert_eq!(
            VapidHeader::parse(VALID_HEADER),
            Ok(VapidHeader {
                scheme: "vapid".to_string(),
                token: TOKEN.to_string(),
                version_data: VapidVersionData::Version2 {
                    public_key: KEY.to_string()
                }
            })
        );
    }
}
