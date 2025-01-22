use std::collections::HashMap;
use std::fmt;

use base64::Engine;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::headers::util::split_key_value;
use autopush_common::util::{sec_since_epoch, ONE_DAY_IN_SECONDS};

pub const ALLOWED_SCHEMES: [&str; 3] = ["bearer", "webpush", "vapid"];
const DEFAULT_SUB: &str = "mailto:";

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct VapidClaims {
    pub exp: u64,
    pub aud: Option<String>,
    #[serde(default = "VapidClaims::default_sub")]
    pub sub: Option<String>,
}

impl Default for VapidClaims {
    fn default() -> Self {
        Self {
            exp: VapidClaims::default_exp(),
            aud: None,
            sub: VapidClaims::default_sub(),
        }
    }
}

impl VapidClaims {
    pub fn default_exp() -> u64 {
        sec_since_epoch() + ONE_DAY_IN_SECONDS
    }

    pub fn default_sub() -> Option<String> {
        Some(DEFAULT_SUB.into())
    }
}

impl fmt::Display for VapidClaims {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VapidClaims")
            .field("exp", &self.exp)
            .field("aud", &self.aud)
            .field("sub", &self.sub)
            .finish()
    }
}

impl TryFrom<VapidHeader> for VapidClaims {
    type Error = VapidError;
    fn try_from(header: VapidHeader) -> Result<Self, Self::Error> {
        let b64_str = header
            .token
            .split('.')
            .nth(1)
            .ok_or(VapidError::InvalidVapid(header.token.to_string()))?;
        let value_str = String::from_utf8(
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(b64_str)
                .map_err(|e| VapidError::InvalidVapid(e.to_string()))?,
        )
        .map_err(|e| VapidError::InvalidVapid(e.to_string()))?;
        serde_json::from_str::<VapidClaims>(&value_str)
            .map_err(|e| VapidError::InvalidVapid(e.to_string()))
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

    /// Return the claimed `sub` after doing some minimal checks for validity.
    /// WARNING: THIS FUNCTION DOES NOT VALIDATE THE VAPID HEADER AND SHOULD
    /// ONLY BE USED FOR LOGGING AND METRIC REPORTING FUNCTIONS.
    /// Proper validation should be done by [crate::extractors::subscription::validate_vapid_jwt()]
    pub fn insecure_sub(&self) -> Result<String, VapidError> {
        // This parses the VAPID header string
        let data = VapidClaims::try_from(self.clone()).inspect_err(|e| {
            warn!("🔐 Vapid: {:?} {:?}", e, &self.token);
        })?;

        if let Some(sub) = data.sub {
            if sub.is_empty() {
                info!("🔐 Empty Vapid sub");
                return Err(VapidError::SubEmpty);
            }
            if !sub.starts_with("mailto:") && !sub.starts_with("https://") {
                info!("🔐 Vapid: Bad Format {:?}", sub);
                return Err(VapidError::SubBadFormat);
            }
            info!("🔐 Vapid: sub: {:?}", sub);
            return Ok(sub.to_owned());
        }
        Err(VapidError::SubMissing)
    }

    pub fn claims(&self) -> Result<VapidClaims, VapidError> {
        VapidClaims::try_from(self.clone())
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
    #[error("Unparsable sub string")]
    SubInvalid,
    #[error("Improperly formatted sub string")]
    SubBadFormat,
    #[error("Empty sub string")]
    SubEmpty,
    #[error("Missing sub")]
    SubMissing,
}

impl VapidError {
    pub fn as_metric(&self) -> &str {
        match self {
            Self::MissingToken => "missing_token",
            Self::InvalidVapid(_) => "invalid_vapid",
            Self::MissingKey => "missing_key",
            Self::InvalidKey(_) => "invalid_key",
            Self::InvalidAudience => "invalid_audience",
            Self::InvalidExpiry => "invalid_expiry",
            Self::KeyMismatch => "key_mismatch",
            Self::FutureExpirationToken => "future_expiration_token",
            Self::UnknownScheme => "unknown_scheme",
            Self::SubInvalid => "invalid_sub",
            Self::SubBadFormat => "bad_format_sub",
            Self::SubEmpty => "empty_sub",
            Self::SubMissing => "missing_sub",
        }
    }
}

#[cfg(test)]
mod tests {

    use crate::headers::vapid::VapidError;

    use super::{VapidClaims, VapidHeader, VapidVersionData};

    // This was generated externally using the py_vapid package.
    const VALID_HEADER: &str = "vapid t=eyJ0eXAiOiJKV1QiLCJhbGciOiJFUzI1NiJ9.ey\
        JhdWQiOiJodHRwczovL3B1c2guc2VydmljZXMubW96aWxsYS5jb20iLCJleHAiOjE3MTM1N\
        jQ4NzIsInN1YiI6Im1haWx0bzphZG1pbkBleGFtcGxlLmNvbSJ9.t7uOYm8nbqFkuNpDeln\
        -UeqSC58xu96Mc9tUVifQu1zAAndHYwvMd3-u--PuUo3S_VrqYXEaIlNIOOrGd3iUBA,k=B\
        LMymkOqvT6OZ1o9etCqV4jGPkvOXNz5FdBjsAR9zR5oeCV1x5CBKuSLTlHon-H_boHTzMtM\
        oNHsAGDlDB6X7vI";

    const VAPID_HEADER_NO_SUB: &str = "vapid t=eyJhbGciOiJFUzI1NiIsInR5cCI6IkpX\
        VCJ9.eyJpYXQiOjE3Mzc1MzIyNjYsImV4cCI6MTczNzUzNjc2NiwibmJmIjoxNzM3NTMyMj\
        Y2LCJhdWQiOiJodHRwOi8vbG9jYWxob3N0In0.162KXmtpTkug2VeCANg4tWjLAkgDn0FiA\
        f89BMNNv5cPi1vIzFclHd6p2xfiqzr-eaUGkPapgzEoEZsVRrNn7Q,k=BOniQ9xHBPNY9gn\
        QW4o-16vHqOb40pEIMifyUdFsxAgyzVkFMguxw0QrdbZcq8hRjN2zpeInRvKVPlkzABvuTn\
        I";

    const VAPID_HEADER_EMPTY_SUB: &str = "vapid t=eyJhbGciOiJFUzI1NiIsInR5cCI6I\
        kpXVCJ9.eyJpYXQiOjE3Mzc1MzMzNTksImV4cCI6MTczNzUzNzg1OSwibmJmIjoxNzM3NTM\
        zMzU5LCJzdWIiOiIiLCJhdWQiOiJodHRwOi8vbG9jYWxob3N0In0.GI7wpI0tGwaOJjlVpd\
        MMaVsb4S3eFTS4txC_WFW_Q4eRaspLfbWjkIzMSzdPo8WKB0UwSaMocZQrU4Omjba4_g,k=\
        BOniQ9xHBPNY9gnQW4o-16vHqOb40pEIMifyUdFsxAgyzVkFMguxw0QrdbZcq8hRjN2zpeI\
        nRvKVPlkzABvuTnI";

    const VAPID_HEADER_WRONG_SCHEME: &str = "vapid t=eyJhbGciOiJFUzI1NiIsInR5cC\
        I6IkpXVCJ9.eyJpYXQiOjE3Mzc1MzM1MDEsImV4cCI6MTczNzUzODAwMSwibmJmIjoxNzM3\
        NTMzNTAxLCJzdWIiOiJzbWI6Ly9ub3RfYWxsb3dlZCIsImF1ZCI6Imh0dHA6Ly9sb2NhbGh\
        vc3QifQ.3NP0HiCQGyW8orAJ0Cc2WaEbPbetox1aU4vS1ndU-n-t-fnln4TurqPOvXD3shV\
        uyT02vYiBBA6cGThGraeOag,k=BOniQ9xHBPNY9gnQW4o-16vHqOb40pEIMifyUdFsxAgyz\
        VkFMguxw0QrdbZcq8hRjN2zpeInRvKVPlkzABvuTnI";

    #[test]
    fn parse_succeeds() {
        // brain dead header parser.
        let mut parts = VALID_HEADER.split(' ').nth(1).unwrap().split(',');
        let token = parts.next().unwrap().split('=').nth(1).unwrap().to_string();
        let public_key = parts.next().unwrap().split('=').nth(1).unwrap().to_string();

        let expected_header = VapidHeader {
            scheme: "vapid".to_string(),
            token,
            version_data: VapidVersionData::Version2 { public_key },
        };

        let returned_header = VapidHeader::parse(VALID_HEADER);
        assert_eq!(returned_header, Ok(expected_header.clone()));

        assert_eq!(
            returned_header.unwrap().claims(),
            Ok(VapidClaims {
                exp: 1713564872,
                aud: Some("https://push.services.mozilla.com".to_owned()),
                sub: Some("mailto:admin@example.com".to_owned())
            })
        );

        // Ensure the parent `.sub()` returns a valid value.
        let returned_header = VapidHeader::parse(VALID_HEADER);
        assert_eq!(
            returned_header.unwrap().insecure_sub(),
            Ok("mailto:admin@example.com".to_owned())
        )
    }

    #[test]
    fn parse_no_sub() {
        let returned_header = VapidHeader::parse(VAPID_HEADER_NO_SUB);
        assert_eq!(
            returned_header.unwrap().insecure_sub(),
            Ok("mailto:".to_owned())
        )
    }

    #[test]
    fn parse_wrong_sub() {
        {
            let returned_header = VapidHeader::parse(VAPID_HEADER_EMPTY_SUB);
            assert_eq!(
                returned_header.unwrap().insecure_sub(),
                Err(VapidError::SubEmpty)
            )
        }
        {
            let returned_header = VapidHeader::parse(VAPID_HEADER_WRONG_SCHEME);
            assert_eq!(
                returned_header.unwrap().insecure_sub(),
                Err(VapidError::SubBadFormat)
            )
        }
    }
}
