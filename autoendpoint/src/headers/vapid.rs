use crate::headers::util::split_key_value;
use std::collections::HashMap;
use thiserror::Error;

const ALLOWED_SCHEMES: [&str; 3] = ["bearer", "webpush", "vapid"];

/// Parses the VAPID authorization header
#[derive(Clone, Debug, PartialEq)]
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
#[derive(Clone, Debug, PartialEq)]
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

#[derive(Debug, Error, PartialEq)]
pub enum VapidError {
    #[error("Missing VAPID token")]
    MissingToken,
    #[error("Missing VAPID public key")]
    MissingKey,
    #[error("Invalid VAPID public key")]
    InvalidKey,
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
