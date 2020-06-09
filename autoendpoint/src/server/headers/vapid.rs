use crate::server::headers::util::split_key_value;
use thiserror::Error;

const ALLOWED_SCHEMES: [&str; 3] = ["bearer", "webpush", "vapid"];

/// Parses the VAPID authorization header
#[derive(Debug, PartialEq)]
pub struct VapidHeader {
    pub scheme: String,
    pub version: usize,
    pub public_key: Option<String>,
}

impl VapidHeader {
    /// Parse the VAPID authorization header. The public key is available if the
    /// version is 2 ("vapid" scheme).
    pub fn parse(header: &str) -> Result<VapidHeader, VapidError> {
        let mut scheme_split = header.splitn(2, ' ');
        let scheme = scheme_split.next().ok_or(VapidError::MissingToken)?;
        let data = scheme_split
            .next()
            .ok_or(VapidError::MissingToken)?
            .replace(' ', "");

        if !ALLOWED_SCHEMES.contains(&scheme) {
            return Err(VapidError::UnknownScheme);
        }

        let (version, public_key) = if scheme == "vapid" {
            // Get the value of the "k" item. This is the public key.
            let (_, public_key) = data
                .split(',')
                .filter_map(split_key_value)
                .find(|(key, _)| *key == "k")
                .ok_or(VapidError::InvalidToken)?;

            (2, Some(public_key.to_string()))
        } else {
            (1, None)
        };

        Ok(Self {
            scheme: scheme.to_string(),
            version,
            public_key,
        })
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum VapidError {
    #[error("Missing auth token")]
    MissingToken,
    #[error("Invalid auth token")]
    InvalidToken,
    #[error("Unknown auth scheme")]
    UnknownScheme,
}

#[cfg(test)]
mod tests {
    use super::VapidHeader;

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
                version: 2,
                scheme: "vapid".to_string(),
                public_key: Some(
                    "BAS7pgV_RFQx5yAwSePfrmjvNm1sDXyMpyDSCL1IXRU32cdtopiAmSysWT\
                     CrL_aZg2GE1B_D9v7weQVXC3zDmnQ"
                        .to_string()
                )
            })
        );
    }
}
